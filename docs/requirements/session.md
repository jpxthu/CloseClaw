# Session 需求

## 概述

Session 模块为 User 提供 Agent 对话上下文的持久化、可恢复与可管理能力——每次对话自动保存，系统重启不丢失历史，闲置自动归档。

## 功能需求

### F1. 对话持久化与恢复

User 与 Agent 的对话自动持久化，已写入对话历史的消息完整保留。系统重启后 Agent 能接续之前的对话。

- User 发送消息时，系统自动查找该对话对应的 Session——若存在则复用，若已归档则恢复，否则创建新 Session
- Session 由平台、发送者、会话对端、账号四个维度共同标识，字段定义见 [im_adapter §F2](im_adapter.md)（入站消息归一化）。相同标识下可以有多个历史 Session，当前活跃的是最后活跃时间最近的一个
- 会话在 Agent 范围内隔离：消息先路由到接收该消息的机器人所绑定的 Agent（见 [gateway §F4](gateway.md)），再按上述四维标识查找 Session
- 归档的 Session 被访问时自动恢复：
  - 若 Session 正在归档中，等待归档完成后自动恢复，恢复时提示 User「会话归档中，稍后恢复…」
  - 若 Session 已归档，恢复时提示 User「正在恢复会话…」
  - 恢复后 system prompt 按最新配置重新注入，详见 F2
- 崩溃恢复详见 F7

> **交叉引用**：新 Session 由 `/new` 指令触发创建，详见 [slash §F3](slash.md)（会话管理）。
> **交叉引用**：系统崩溃后的恢复流程详见 [F7](#f7-运行健康与安全)（运行健康与安全）。

### F2. 恢复时的 system prompt 重建

Session 恢复时触发 Agent 的 system prompt 重新注入。

- User 追加的 system prompt 自定义指令持久化保存，归档恢复后完整保留

> **交叉引用**：完整触发事件清单、缓存失效策略与文件变更的自动反映，详见 [system_prompt §F6](system_prompt.md)（内容缓存与自动刷新）。
> **交叉引用**：技能清单的格式和技能文件变更的生效规则详见 [skills §F4](skills.md)（技能清单）、[skills §F5](skills.md)（技能文件变更）。
> **交叉引用**：追加指令的交互方式（/system add/list/clear）详见 [slash §F6](slash.md)（system prompt 追加）。
> **交叉引用**：bootstrap 文件的清单与注入顺序，详见 [system_prompt §F1](system_prompt.md)（身份与行为准则定义）。
> **交叉引用**：子 Session 的文件加载范围，详见 [system_prompt §F8](system_prompt.md)（会话类型适配）。

### F3. 长对话压缩

当对话历史接近上下文窗口上限时，系统自动将对话压缩为结构化摘要，释放 token 空间以继续对话。User 也可以手动触发压缩。

- 手动压缩：User 通过 `/compact` 指令触发，可附带自定义保留指令，指导压缩引擎重点保留哪些内容
- 自动压缩：每轮对话后检测 token 用量
  - 告警阶段：剩余空间低于告警阈值时，提示 User 即将压缩。若后续剩余空间回升至告警阈值以上，告警自动取消
  - 压缩阶段：剩余空间低于压缩阈值时，自动执行压缩
  - 告警阈值大于压缩阈值（告警先于压缩触发），两个阈值均按上下文窗口百分比配置，每个 Agent 可独立设置
- 压缩只处理 User 与 Agent 的对话消息，system prompt 内容完整保留
- 压缩结果为一条结构化摘要消息，覆盖六个维度：Goal / Constraints & Preferences / Progress / Key Decisions / Next Steps / Critical Context
- 连续压缩失败后自动进入保护暂停（仅阻止自动压缩再次触发，不影响活跃判定和归档），手动 `/compact` 成功后自动解除保护暂停
> **交叉引用**：手动压缩由 `/compact` 指令触发，详见 [slash §F5](slash.md)（上下文压缩）。
> **交叉引用**：压缩前自动备份的通用机制详见 [F7](#f7-运行健康与安全)（运行健康与安全）。

### F4. 子 Session 委托与协调

Agent 可以将子任务委托给子 Session，并行委托多个子 Session，等待结果后继续决策。

- Agent 可以创建子 Session 来委托子任务。子 Session 的任务描述注入到 system prompt 中，不属于对话消息，压缩时不受影响
- Agent 可以向已有未完成的子 Session 发送新任务
- Agent 可以终止子 Session，级联终止其所有后代
- 子 Session 完成后结果通过统一消息队列注入父 Session（带去重保护）。子 Session 完成通知与其他消息类型（User 消息、后台任务结果、系统通知）一视同仁，统一按 F10 的排队规则处理，Agent 不需要轮询
- 终止当前 Session 时，所有子 Session 级联终止
- 子 Session 超时通知：
  - 子 Session 的运行时长达到设定的超时上限时，系统向父 Session 注入超时通知。通知内容包含：设定的超时时间、实际运行时长、上下文窗口使用情况及 token 用量
  - 若父 Agent 未终止该子 Session，系统在超时时间的 50% 间隔后再次注入通知，循环往复。间隔比例可配置
  - 父 Agent 收到通知后自行决定：终止子 Session、继续等待、或向 User 汇报
  - 子 Session 的超时时间来源及优先级，详见 [agent §F7](agent.md)（子 Agent 创建）。
- 父 Session 每轮对话开始时，系统注入当前活跃子 Session 摘要：正在执行的子 Session 数量及每个子 Session 的概要信息（Agent 标识、任务简述、已运行时长）

> **交叉引用**：子 Session 结果注入与排队规则详见 [F9](#f9-消息注入)（消息注入）、[F10](#f10-消息排队)（消息排队）。
> **交叉引用**：子 Session 完成通知的送达保证和僵死检测详见 [F7](#f7-运行健康与安全)（运行健康与安全）。
> **交叉引用**：`/stop` 指令触发 Session 终止，详见 [slash §F3](slash.md)（会话管理）。

### F5. 推理强度控制

User 控制 LLM 调用的推理强度。

- 推理强度的设置在会话内持续生效

> **交叉引用**：推理强度档位定义、默认值、优先级和模型能力降级策略详见 [llm §F4](llm.md)（推理强度控制）。
> **交叉引用**：运行时设置由 `/reasoning` 指令完成，详见 [slash §F10](slash.md)（推理强度控制）。
> **交叉引用**：`/verbose` 指令控制信息展示等级，详见 [slash §F11](slash.md)（展示等级）。

### F6. 会话归档与清理

inactive 的 Session 自动归档，User 无需手动管理。User 可配置归档数据的自动清理时间，默认不自动删除。

- inactive 的 Session 自动归档：标记为归档状态，不再作为活跃 Session
- inactive 判定条件：四维活跃维度均为否（详见 F11），且距上次 User 活动超过配置的 inactive 时长
- User 配置清理时间后，已归档超过该时长的 Session 彻底删除（元数据 + 对话记录）
- 每个 Agent 可独立配置 inactive 时长和清理时间，主 Session 与子 Session 可以分别设置
- 各配置项独立回退：未配置的项使用系统默认值（inactive 30 分钟归档、归档数据不自动删除）
- 新会话创建时使用当前配置；已在运行的 Session 沿用创建时的配置，不随配置变更而变
- 归档与清理扫描参数（扫描间隔等）变更后自下一次扫描起生效
- 有未完成的子 Session 时，父 Session 不被判定为 inactive，不触发归档。若因系统错误被归档，记录告警日志，该子 Session 的完成通知丢弃
- 系统对活跃 Session 和对话记录做双向一致性校验——有活跃 Session 标记但无对应对话记录，视为损坏并清理；有对话记录但无对应 Session 标记，视为孤立并清理

> **交叉引用**：会话配置的重载机制详见 [config §F4](config.md)（配置重载）。

### F7. 运行健康与安全

Agent 对话过程中，系统自动检测异常并提供保护机制，防止对话上下文损坏。

- 每轮对话结束后自动检测：响应超时、空响应、结构化异常等问题
- 可配置的自动质量检查：检测 Agent 是否陷入工具调用死循环
- 可配置的自动质量检查：检测 Agent 是否只计划不执行
- 对涉及对话历史完整性的操作（压缩、system prompt 修改），执行前自动创建对话备份，操作失败时可回滚到上一个安全状态
- 系统崩溃时，自动识别未完成的工具调用、子 Session 委托和未发送的出站消息。未发送的出站消息自动重投递；其余操作注入恢复通知，由 Agent 自行决定如何处理
- 系统定时扫描子 Session（F4 的超时通知不排斥本项僵死兜底——父 Agent 选择继续等待的，仍受僵死判定保护）：
  - 已完成（四维活跃维度均为否且已产出最终 assistant 消息）但完成通知未成功送达父 Session 的，补推完成通知（若父 Session 已归档则跳过）
  - 非已完成状态且超过五分钟无新产出（无新 assistant 消息、无工具执行结果变化）的，判定为僵死，自动终止（级联终止其所有后代）并向父 Session 注入僵死通知（若父 Session 已归档则跳过）



### F8. 工作目录

每个 Session 拥有独立的工作目录，作为该会话文件操作的默认路径，随会话生命周期初始化和重置。

- 工作目录在 Session 创建时初始化为默认值，恢复时重置为默认值

> **交叉引用**：工作目录的查看与变更由 `/pwd`、`/cd`、`/git` 指令完成，详见 [slash §F7](slash.md)（工作目录操作）。

### F9. 消息注入

后台任务、记忆注入和子 Session 结果注入消息队列，Agent 在后续轮次中按常规对话流程处理。

- 后台任务完成时，结果按优先级（now > next > later）注入消息队列
- 记忆注入通过 Session 的消息通道完成，具体内容与注入位置由 memory 模块定义
- 记忆注入与后台消息注入互不冲突，可共存于同一批消息

> **交叉引用**：子 Session 结果的注入由 [F4](#f4-子-session-委托与协调)（子 Session 委托与协调）定义。
> **交叉引用**：记忆搜索结果的注入位置规则详见 [memory §F4](memory.md)（对话中自动注入相关记忆）。

### F10. 消息排队

User 消息按以下阻塞规则分派。判定条件使用 F11 的四维活跃维度。

排队条件 = LLM 推理为是 OR 同步工具等待结果为是。

- 满足排队条件时：User 消息按序排队，排队时提示 User「⏳ 正在排队…」
  - LLM 推理结束后注入排队消息
  - 同步工具结果返回后，工具结果与排队消息同批注入
- 不满足排队条件时：User 消息立即注入——无论后台任务和子 Session 是否活跃。Session 通过后台任务完成通知和子 Session 完成通知提醒 Agent 有待处理的后台任务，Agent 自行决定如何应对
- 非 User 消息（子 Session 完成通知、后台任务结果、记忆注入等）与 User 消息沿用同一阻塞框架：
  - 满足排队条件时：同批积压的非 User 消息（按到达时间先后）优先注入，随后注入排队中的 User 消息
  - 不满足排队条件时：非 User 消息立即注入

> **交叉引用**：斜杠指令的排队/立即语义由 Gateway 路由决策决定，详见 [gateway §F5](gateway.md)（斜杠指令拦截与分派）。

### F11. Session 活跃维度

Session 在任意时刻可以在多个维度上同时处于活跃状态，每个维度独立开启或关闭。

- LLM 正在推理或流式输出
- Agent 调用了工具并等待其返回结果以继续推理（同步调用）
- Agent 异步调用了工具（后台任务），不阻塞当前推理流程
- Session 有未完成的子 Session（已创建但未完成）

四维活跃维度的复合判定由各功能域按需组合：

- 消息分派：由 F10 定义判定条件
- 归档判定：由 F6 定义判定条件
- Workflow 验收：由 [workflow §F3](workflow.md)（步骤引导执行）定义判定条件
- 优雅关闭：由 [daemon §F2](daemon.md)（优雅关闭）定义判定条件
- 斜杠指令路由：由 [gateway §F5](gateway.md)（斜杠指令拦截与分派）定义判定条件

### F12. 调试日志

Session 模块在以下环节记录调试日志：
- 会话创建、查找、归档恢复
- 对话历史的追加与修改
- 上下文压缩事件（触发原因、压缩结果概要）
- 活跃维度变化（任一维度开启或关闭）
- 子 Session 创建与完成
- 消息注入事件（后台任务结果、记忆注入）
- 健康检查与异常检测结果

> **交叉引用**：日志框架定义（格式、级别、追踪标识、存储轮转、隐私脱敏）详见 [debug_log](debug_log.md)。

## 关联设计文档

- [session/README.md](../design/session/README.md)
- [session/session-lifecycle.md](../design/session/session-lifecycle.md)
- [session/session-execution.md](../design/session/session-execution.md)
- [session/session-injection.md](../design/session/session-injection.md)
- [session/working-directory.md](../design/session/working-directory.md)
- [session/compact-process.md](../design/session/compact-process.md)
- [session/llm-session-enhancements.md](../design/session/llm-session-enhancements.md)
- [session/session-tools.md](../design/session/session-tools.md)
- [session/run-health.md](../design/session/run-health.md)
- [session/session-recovery.md](../design/session/session-recovery.md)
- [session/spawn-tree.md](../design/session/spawn-tree.md)

## 非功能需求

> 复杂度符号：A = 当前活跃 Session 数量，N = 历史会话总量（含已归档）。

- **可靠性**：对话记录在系统重启或进程异常退出后完整保留，已写入历史的消息不丢失。正在执行的操作（工具调用、子 Session 委托、出站消息）在崩溃后能被识别——未发送的出站消息自动重投递，其余操作注入恢复通知，由 Agent 决定后续处理
- **可恢复性**：系统重启后自动恢复所有活跃 Session。恢复耗时复杂度 O(A)，与 N 无关
- **性能**：Agent 回复实时逐字展示。后台维护任务（归档扫描）不阻塞 User 对话的响应
- **可配置性**：每个 Agent 的 inactive 时长、清理时间可独立配置，主 Session 与子 Session 可以分别设置；各配置项独立回退到系统默认值。配置变更的生效时机见 F6 与 [config §F4](config.md)（配置重载）
- **会话独立性**：会话路由、委派、归档恢复等日常操作对 N 的复杂度为 O(1) 或 O(log N)，不对全量历史会话做 O(N) 遍历
- **长期运行稳定性**：系统累计委派大量子任务后，委派新子任务和 User 对话的响应速度不随已完成子任务数量增加而退化。已完成子任务的结果归档后仍可查，但不持续占用运行资源；内存占用 O(A)，对 N 为 O(1)
