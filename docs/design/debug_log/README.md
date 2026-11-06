# Debug Log

## 概述

- 关联需求文档：[requirements/debug_log.md](../../requirements/debug_log.md)
- 一句话：为系统内部行为提供统一的调试日志基础设施——消息追踪与子调用关联、分级过滤、凭据脱敏、JSONL 日文件写入与保留清理，使运维 Agent 以追踪标识把一条消息在各模块的事件串联成完整链路，结合代码定位问题根因。

## 架构

调试日志框架（下称「框架」）是基础层横切设施，以库的形式由各进程宿主初始化：daemon 与 CLI 进程在启动时各自创建框架实例，传入日志目录、日志配置（见「日志配置与存储位置」）与该进程可获得的凭据值（见「核心组件 · RedactionEngine」）。同一进程内的各模块共用一个实例；多个进程使用同一日志目录、追加同一自然日文件。本模块只承担一个内聚能力（统一的调试日志设施），各组件不可独立使用，故不另立子功能文档。

框架是所有调试事件的统一落盘通道：各模块在消息处理链路或自身操作的关键节点产出事件并交给框架，由框架完成分级过滤、脱敏与持久化。产出方模块直接调用本模块的事件投递与 trace 取用接口——本模块属基础层，可为各层直接依赖，不需经 common trait 中转（crate 依赖见 [STANDARDS.md 依赖方向允许边表](../STANDARDS.md)）。

框架不替代各模块已有的其它日志形态：

- Session 的对话历史 checkpoint（含出站交付记录）是会话持久化数据，服务于上下文恢复与投递重放
- 各模块经系统日志（tracing，进程标准错误输出）输出的运行时日志，关心进程运行状态
- permission 模块的文件审计日志（结构化审计条目，经 common AuditLogger 落盘）服务于审计与合规查询，按审计需求定义条目与保留，不经本框架

以上形态与调试日志用途不同，各自独立存储：模块把需要统一追踪与分级的调试事件交给框架记录与呈现，既有日志机制中的记录不被复制进框架日志。

### 核心组件

**TraceContext**：追踪上下文管理。每条日志事件携带 trace_id 与 span_id，子调用事件另带 parent_span_id。

- 入站消息的 trace 在消息到达后由接收该消息的适配器（各 IM 平台插件与 terminal 渠道插件）在解析阶段建立：框架分配 trace_id，写入 [NormalizedMessage.trace_id](../common/shared-types.md#normalizedmessage)；建立失败时为空串
- 消息的根 span 在 Gateway 完成入站处理后（写入 metadata 的同一时点）经框架建立，trace_id 与 span_id 一并写入 metadata（键与写入时序见 [common 元数据键](../common/shared-types.md#processedmessage)），随后沿处理链路继续传递；入站链内的事件只有 trace_id（此阶段根 span 尚未建立，span_id 为空串）。框架不维护 trace_id 到 span 的映射表，根 span 标识仅随 metadata 与调用上下文传递
- trace 上下文随调用链显式传递，不依赖全局隐式状态：产出事件的模块从所处处理上下文取得当前 trace 上下文——Gateway 链后之前取 NormalizedMessage.trace_id（此时根 span 尚未建立，span_id 记为空串），链后取 metadata 中的 trace_id 与 span_id，子调用链取调用方传入的当前 span 上下文；需要派生子 span 时经框架接口派生并填入 parent_span_id
- 主链路触发子调用（LLM 调用工具执行、spawn 子 Session 等）时，从当前 span 派生子 span：同一进程内的子调用由调用方传入当前 span 上下文；跨 Session 的子调用（spawn）由 spawn 请求携带父 trace_id 与父 span_id，子 Session 沿用父 trace_id，其首个 span 以父 span 为 parent_span_id，使子链路日志并入同一 trace
- span_id 在所属 trace 内唯一（由框架保证），parent_span_id 据此可解析；trace_id 跨进程与进程重启后均不重复
- 非消息事件（定时任务、后台任务、CLI 进程的启动检查与命令执行等）由触发模块经框架的 trace 创建入口建立独立 trace，建立 trace 时一并建立根 span（span_id 非空、parent_span_id 为空），触发模块标识作为事件的来源模块
- trace_id 由微秒级时间戳、进程实例标识与进程内单调递增的 trace 序号组合：进程实例标识在进程启动时生成一次，使多进程并存与进程重启后不产生相同标识；trace 序号区分同一微秒内的多条消息，保证正常情况下每条入站消息的标识唯一。trace 序号与 LogEvent 的事件序号是两个独立计数器：前者标识消息的 trace，后者标识日志事件的先后
- trace 建立或传播失败不阻断消息处理：缺 trace 上下文的事件仍照常记录（trace_id 与 span_id 记为空串），并记一条降级告警事件，链路允许不完整（异常口径见 [processor_chain §F2](../../requirements/processor_chain.md#f2-消息追踪标识)）

**LogLevel（5 级）**：定义日志事件的严重程度，从低到高为完整内容（`trace`）-> 中间状态（`debug`）-> 关键事件（`info`）-> 降级告警（`warn`）-> 异常（`error`）；括号内标识用于配置取值与事件级别字段。框架记录最低记录级别及更高级别的事件。

各级别语义：

- **异常**：功能不可用或受损，必须人工关注。如 session 创建失败致消息无法处理、LLM 调用连续重试后仍失败
- **降级告警**：功能降级但可用，需要关注但不紧急。如上下文窗口接近上限、消息注入非关键路径异常、追踪标识建立异常、缓存命中一致性校验判定为缓存异常
- **关键事件**：消息生命周期关键节点。如消息到达、路由决策、LLM 调用起止（含供应商、模型、token 用量与耗时）、工具执行起止、发送完成
- **中间状态**：处理过程中的中间步骤和判定结果。如追踪标识建立结果、session 查找匹配结果、权限检查通过或拒绝、内容过滤详情
- **完整内容**：用于深度排查的完整数据。如入站原始消息全文、LLM 完整请求与响应（含 system prompt 和 tool definitions）、工具完整参数和返回值、出站消息全文。该级默认不落盘，深度排查时把最低记录级别调至该级才记录

凭据类敏感数据在任何级别均不记录明文（见 RedactionEngine）。

**LogEvent 结构化事件**：每条日志事件包含 trace_id、span_id、parent_span_id（子调用时有值）、session_key、微秒级时间戳、进程实例标识、进程内单调递增的事件序号、级别、来源模块、事件类型、结构化负载。session_key 在事件产出方取得该消息 session_key 时携带，否则为空串（入站原文事件早于 session_key 计算）；trace_id 是链路检索主键，session_key 仅作该消息的会话上下文记录。事件先后：同一进程内按事件序号，跨进程按微秒级时间戳，时间戳相同时按文件追加顺序。格式为 JSONL——一行一条 JSON 对象，对程序和人类均可读。事件类型以「模块.事件」形式命名（如 `gateway.inbound.arrived`），在各模块间不重名；负载为该事件的结构化字段。

**LevelFilter**：分级过滤。比对该事件级别与当前最低记录级别：低于最低记录级别的事件丢弃，不进入脱敏与写入。框架自身产生的异常与降级告警事件同样经过滤——最低记录级别高于降级告警时，追踪异常等告警不落盘，此时链路缺口不可从日志辨识（与分级语义一致）。

**RedactionEngine**：凭据脱敏引擎。写入前扫描事件负载，替换为脱敏标记、不记录明文：①结构化负载中的敏感字段名（API Key、Token、Secret、Authorization 等；该清单由框架内置，不需配置）；②系统持有的凭据字面值——包括宿主注入的供应商凭据与各模块自行管理的平台 token：启动时由宿主注入，运行中新取得的凭据（如 CLI 配置向导写入、平台 token 续期）由持有方立即追加登记，下次写入前即生效；凭据值在负载任意位置（含入站原文、完整响应、工具返回值等自由文本）出现即替换。用户正文中出现的非系统凭据内容原样保留，以支持完整内容级别的排查。脱敏在序列化为 JSON 之前完成。

**LogWriter**：日志写入器。负责当日文件的追加写入与即时落盘（不做批量缓冲），使日志产生即持久化；单条事件以一次追加写入完成（含行尾换行），多个进程追加同一文件时不出现单行截断或交错。每次写入前确认当日文件存在，被手动删除时即时重建。写入失败不阻塞消息正常流转：失败期间经系统日志上报为系统异常（进程在恢复前退出时，失败痕迹仅留存于系统日志），写入恢复后的首条日志中补记一条「日志写入失败」异常事件（含失败次数与持续时长），使运维 Agent 仅凭日志文件也能发现该故障。

**LogRetention**：日志保留策略。自然日以系统本地时区为准，日志写入单个日文件，文件名含该自然日（如 `debug-2026-08-07.jsonl`），文件年龄以文件名中的日期为准。超过保留天数的文件自动删除；删除幂等、多进程并发执行安全，故各进程在轮转时均可执行保留检查，daemon 启动时另执行一次。手动清理由 Owner 直接删除日志文件完成：按文件名日期选定指定时间范围或全部，范围仅限框架自身的日志文件，不受保留天数约束、即时生效；当日文件被删后由下一次写入重建。

### 日志配置与存储位置

- **配置来源**：`config/debug_log.json`，由 config 模块加载并校验（校验规则见 [config 模块](../config/README.md)）；宿主创建框架实例后向 config 模块登记，config 在加载与重载后把配置值推送给已登记的实例——框架属基础层，不反向依赖 config 模块
- **可配置项**：最低记录级别（默认 `debug`，即中间状态）、保留天数（默认 7 天）
- **缺失与生效边界**：配置文件或字段缺失时按默认值运行，不关闭日志；最低记录级别自下一条日志起生效，保留天数自下一次清理起生效（需求 [config §F4](../../requirements/config.md#f4-配置重载)）。daemon 与 CLI 进程内的框架实例均在获得新配置值后自下一条日志起采用
- **日志目录**：`<配置根目录>/logs/`——配置根目录经 [platform 配置目录](../platform/config-directory.md) 解析后由进程宿主传入，框架确保该目录存在；目录与日志文件权限仅 Owner 可访问，日志文件不区分 User。该目录不在任何 Agent 的 workspace 内，产品内 Agent 经工具读取时按权限模块的读维度规则判定（边界外路径不自动放行，不设特例）；运维 Agent 是 Owner 在本机运行的排障工具，其读取属本系统之外的访问，不计入权限与沙盒判定

## 数据流

1. 各模块在消息处理链路或自身操作节点产出日志事件，携带 trace_id、span_id（非消息事件先经框架建立独立 trace，见「核心组件 · TraceContext」）
2. LevelFilter 比对事件级别与最低记录级别：低于则丢弃，否则继续
3. RedactionEngine 按敏感字段名与系统凭据值扫描负载：命中则替换为脱敏标记，否则透传
4. LogWriter 序列化为 JSONL -> 追加写入当日文件 -> 即时落盘；写入失败经系统日志上报，恢复后补记异常事件（见「核心组件 · LogWriter」）

### 追踪标识传播路径

1. IM 平台事件或 CLI 消息到达
2. 接收适配器解析时建立 trace：框架分配 trace_id，写入 NormalizedMessage.trace_id
3. Processor Chain 入站：产出事件时携带 NormalizedMessage.trace_id；session_key 由 SessionRouter 计算后写入 metadata（标识语义与算法见 [processor_chain 入站链路](../processor_chain/inbound-chain.md#session_key-算法)）
4. Gateway 入站处理：建立该消息的根 span，把 trace_id 与 span_id 写入 metadata，随后沿处理链路传递
5. Gateway 路由 -> Session -> LLM 调用 / 工具执行：子调用从当前 span 派生子 span，事件带 parent_span_id
6. 出站记录：Processor Chain 出站链的 OutboundRawLog，或简化出站路径中由 Gateway 直接触发的出站记录，产出出站消息全文事件（完整内容级别）
7. Gateway 渲染调度 -> 适配器渲染 -> 发送
8. 链路中 trace 建立或传播异常：缺上下文的事件照常记录并记降级告警，链路允许不完整

trace_id 是链路检索主键；session_key 记录事件所属消息的会话上下文，它是 Processor Chain 产出的消息级标识（每条消息唯一，含时间戳），不是 Session 的标识、不用于 Session 查找或创建，也不用于按会话聚合日志

### 日志文件生命周期

1. 进程当日首条事件写入时打开（必要时创建）当日文件，多个进程追加同一文件
2. 跨日首条事件写入时切换到新日期文件
3. 切换时与 daemon 启动时执行保留检查（规则见「核心组件 · LogRetention」）
4. 当日文件被清理后，下一事件写入时重建

## 模块关系

> 「上游/下游」指数据流与调用关系（含经 common trait 完成的调用），不等于 crate 依赖；crate 依赖以 [STANDARDS.md 依赖方向允许边表](../STANDARDS.md) 为准。

- **上游**：产出调试事件的各业务模块（Gateway、Processor Chain、Session、LLM、IM Adapter、Tools、Slash、Permission、Memory、System Prompt 等），在各自处理节点产出事件并携带追踪上下文；IM Adapter（平台插件入站解析、出站渲染与发送事件；terminal 渠道插件实现在 CLI 模块、运行在 daemon 进程内，其事件由 daemon 侧实例落盘）；CLI（启动时创建本进程的框架实例并注入凭据；进程内的启动检查、命令执行、配置校验、daemon 启停等事件）；daemon（启动时初始化框架并传入日志目录、日志配置与凭据值，启动时触发一次保留检查）；config（加载并推送日志配置，见「日志配置与存储位置」）
- **下游**：文件系统（日志文件写入与删除）。运维 Agent 读取日志文件，以 trace_id 串联事件链路定位根因，因写入失败或追踪异常而缺失的节点自然不出现
- **无关**：无模块级无关项；与其它可观测通道（系统日志 tracing、permission 审计日志、运营指标上报）及 Session checkpoint 的边界见「架构」
- **共享类型**：本模块对外提供的契约为 LogLevel 与 LogEvent——按 [STANDARDS.md「禁止二次出口」节](../STANDARDS.md)，基础层（platform / debug_log）不受 common 唯一出口约束，故这两个类型定义在本模块、由消费方直接依赖基础层 crate 引用，不进 common；trace_id、span_id、session_key 的字段定义仍在 [common 共享类型](../common/shared-types.md)
