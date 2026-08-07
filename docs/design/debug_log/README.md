# Debug Log

## 概述

- 关联需求文档：[requirements/debug_log.md](../requirements/debug_log.md)
- 核心职责：为系统内部行为提供统一的调试日志基础设施——包含追踪标识传播、分级过滤、JSONL 文件写入和日轮转。各模块通过框架记录日志事件，运维 Agent 以追踪标识串联事件链路定位问题根因。

## 架构

调试日志框架是横切模块——不独立处理业务逻辑，为其他模块提供统一的日志记录能力。各业务模块（Gateway、Processor Chain、Session、LLM、IM Adapter、Tools 等）通过框架记录日志事件，框架负责格式化、过滤、脱敏和持久化。

框架不替代各模块已有的日志机制：
- Session 的对话历史 checkpoint 文件独立于框架
- 模块自身通过系统日志（tracing）输出的运行时日志独立于框架，关心进程运行状态，与调试日志关心消息业务行为的目标不同
- 各模块自身已有的日志机制（如 Session checkpoint）中已覆盖的调试事件，框架从模块日志中读取并应用统一的追踪标识、分级体系、脱敏规则后呈现给运维 Agent，不重复写入框架日志文件——同一事件仅保留在模块自有日志中，框架在消费时做归一化处理

### 核心组件

**TraceContext**：追踪上下文管理。为每条日志事件管理 span 标识分配——主消息链路事件分配根 span，子调用事件从父 span 派生新 span。每条消息的 trace_id 由 IM Adapter 在 webhook 到达时生成（由平台标识、到达时间戳、随机数组合得出），写入消息 metadata 后随消息在各模块间流转。无入站消息的系统内部事件（定时任务、后台任务等）由触发模块自行生成独立 trace_id。当主处理链路中触发子调用（如 LLM 调用工具执行、spawn 子 Agent）时，TraceContext 从当前 span_id 派生子 span 标识，子调用日志通过 parent_span_id 关联回主链路。

**LogLevel（5 级）**：定义日志事件的严重程度，从低到高为完整内容 -> 中间状态 -> 关键事件 -> 降级告警 -> 异常。Owner 通过配置文件配置最低记录级别（默认"中间状态"），框架记录该级别及更高级别的事件。

各级别语义：
- **异常**：功能不可用或受损，必须人工关注。如 session 创建失败致消息无法处理、LLM 调用连续重试后仍失败
- **降级告警**：功能降级但可用，需要关注但不紧急。如上下文窗口接近上限、消息注入非关键路径异常
- **关键事件**：消息生命周期关键节点。如消息到达、路由决策、LLM 调用起止、工具执行起止、发送完成
- **中间状态**：处理过程中的中间步骤和判定结果。如 session 查找匹配结果、权限检查通过/拒绝、中间件拦截判定、内容过滤详情
- **完整内容**：用于深度排查的完整数据。如入站原始消息全文、LLM 完整请求与响应（含 system prompt 和 tool definitions）、工具完整参数和返回值、出站消息全文

凭据类敏感数据在任何级别均不记录明文（见 RedactionEngine）。

**LogEvent 结构化事件**：每条日志事件包含 trace_id、span_id、parent_span_id（子调用时有值）、session_key（消息链路事件携带，非消息链路事件可为空）、毫秒级时间戳、级别、来源模块、事件类型、结构化负载。格式为 JSONL——一行一条 JSON 对象，对程序和人类均可读。

**RedactionEngine**：凭据脱敏引擎。在写入前扫描事件负载中的敏感字段（API Key、Token 等），匹配到的值替换为脱敏标记，不记录明文。脱敏在序列化为 JSON 之前完成。

**LogWriter**：日志写入器。负责 JSONL 格式化、写入当日文件、即时落盘。写入失败不阻塞消息正常流转，但写入失败本身经系统日志（tracing）上报为系统异常——与"框架不替代系统日志"不矛盾，因系统日志是日志事件写入失败后的降级上报通道。

**LogRetention**：日志保留策略。按自然日将日志写入单个文件，文件名含日期。保留天数可由 Owner 通过配置文件配置；超过保留天数的文件自动删除（轮转时触发检查）。Owner 可手动触发日志清理（删除指定时间范围或全部文件），清理范围仅限框架自身的日志文件，不影响各模块自有日志。

## 数据流

日志事件处理流程：
1. 各模块在消息处理链路特定节点产生日志事件
2. 事件携带 trace_id（webhook 消息由 IM Adapter 在接收时生成；非 webhook 事件由触发模块生成）和 session_key（来自 Processor Chain 产出的 metadata）
3. TraceContext 为事件分配 span_id（根 span 或子 span），子调用时填入 parent_span_id
4. LevelFilter 比对该事件级别与配置的最低记录级别
   - 低于配置级别 -> 丢弃，不写入
   - 达到或超过配置级别 -> 继续
5. RedactionEngine 扫描负载中的敏感字段
   - 匹配到 -> 替换为脱敏标记，继续
   - 未匹配 -> 透传，继续
6. LogWriter 将事件序列化为 JSONL -> 写入当日文件 -> 即时落盘
   - 写入成功 -> 完成
   - 写入失败 -> 经系统日志（tracing）上报为系统异常，不影响消息正常流转

### 追踪标识传播路径

1. IM 平台 webhook 到达
2. IM Adapter 接收阶段：生成 trace_id，写入消息 metadata
3. IM Adapter 解析 -> NormalizedMessage（携带 trace_id）
4. Processor Chain 入站：SessionRouter 计算 session_key 写入 metadata（与 trace_id 并存）。session_key 是 Processor Chain 入站阶段计算的消息级追踪标识，由 SessionRouter 基于路由字段和系统时间戳计算得出——命名为 SessionRouter 是因为它在路由过程中顺带计算 session_key，仅用于日志追踪和 session 上下文关联，不改变路由结果，本身不参与 session 路由
5. Gateway 路由 -> Session -> LLM 调用
   - 子调用（如 LLM 调用工具执行、spawn 子 Agent）：TraceContext 从当前 span_id 派生子 span
6. Processor Chain 出站（VerbosityFilter -> DslParser -> 出站调试日志，以上组件由 Processor Chain 模块定义）
7. Gateway 渲染调度 -> IM Adapter 渲染 -> 发送

trace_id 在 webhook 到达即产生，贯穿整条消息链路。日志事件同时携带 trace_id 和 session_key——trace_id 用于端到端消息链路串联，session_key 用于日志追踪和 session 上下文关联。

### 日志文件生命周期

1. 当日首条日志写入时创建新日志文件（如 debug-2026-08-07.jsonl）
2. 当日持续写入该文件
3. 次日首条日志写入时轮转：关闭前一日文件，创建新文件
4. 轮转时触发保留检查：删除最后修改时间超过配置保留天数的文件
5. 手动清理：Owner 可通过接口触发，删除指定时间范围或全部框架日志文件（仅限框架自身的日志文件，不含各模块自有日志）

### 非 webhook 事件的 trace_id

不经过 webhook 的系统内部事件（如定时任务触发、后台任务检查）由触发模块自行生成独立 trace_id——从系统时间戳与模块标识组合产生。此类事件无 session_key（session_key 字段为空）。

## 模块关系

- **上游**：所有需要记录调试日志的业务模块（Gateway、Processor Chain、Session、LLM、IM Adapter、Tools、Slash、Permission 等）。各模块调用框架接口产出日志事件。trace_id 由 IM Adapter 在 webhook 到达时生成并随消息传递；非 webhook 事件由触发模块自行生成
- **下游**：文件系统（日志文件写入）。运维 Agent 读取日志文件以 trace_id 串联事件链路定位根因，链路中因写入失败而缺失的节点自然不出现
- **无关**：系统日志（tracing 输出的运行时日志，关心进程运行状态，与调试日志关心消息业务行为的目标不同。写入失败时系统日志作为降级上报通道，非调试日志框架自身组件）、Session 日志（checkpoint 文件由 Session 模块独立管理，框架不替代）
- **共享类型**：LogLevel 枚举与 LogEvent 事件结构由本模块定义，各消费模块按此约定接入框架
