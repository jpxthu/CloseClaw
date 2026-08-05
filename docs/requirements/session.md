# Session 需求

## 概述

Session 模块为用户提供 Agent 对话上下文的持久化、可恢复与可管理能力——每次对话自动保存，系统重启不丢失历史，闲置自动归档。

## 功能需求

### F1. 对话持久化与恢复

用户与 Agent 的对话自动持久化，已写入对话历史的消息完整保留。系统重启后 Agent 能接续之前的对话。

- 用户发送消息时，系统自动查找该对话对应的 Session——若存在则复用，若已归档则恢复，否则创建新 Session
- Session 由平台、会话发起方、对端、账号四个维度共同标识。相同标识下可以有多个历史 Session，当前活跃的是最后活跃时间最近的一个
- 归档的 Session 被访问时自动恢复：
  - 若 Session 正在归档中，等待归档完成后自动恢复，恢复时提示用户「会话归档中，稍后恢复…」
  - 若 Session 已完成归档，恢复时提示用户「正在恢复会话…」
  - 恢复后 Agent 的 system prompt 按最新配置重新注入
- 崩溃恢复详见 F7

> **交叉引用**：新 Session 由 `/new` 指令触发创建，详见 [slash §F3](slash.md)（会话管理）。
> **交叉引用**：系统崩溃后的恢复流程详见 [F7](#f7-运行健康与安全)（运行健康与安全）。

### F2. 会话恢复

Session 恢复时触发 Agent 的 system prompt 重新注入，注入内容反映当前最新的 bootstrap 文件、Skill 和工具定义。

- 用户追加的 system prompt 自定义指令持久化保存，归档恢复后完整保留

> **交叉引用**：完整触发事件清单、缓存失效策略与文件变更的自动反映，详见 [system_prompt §F6](system_prompt.md)（内容缓存与自动刷新）。
> **交叉引用**：技能清单的格式和热重载机制详见 [skills §F4](skills.md)（技能清单）、[skills §F5](skills.md)（热重载）。
> **交叉引用**：追加指令的交互方式（/system add/list/clear）详见 [slash §F6](slash.md)（system prompt 追加）。
> **交叉引用**：bootstrap 文件的清单与注入顺序，详见 [system_prompt §F1](system_prompt.md)（身份与行为准则定义）。
> **交叉引用**：子 Session 的文件加载范围，详见 [system_prompt §F8](system_prompt.md)（会话类型适配）。

### F3. 长对话压缩

当对话历史接近上下文窗口上限时，系统自动将对话压缩为结构化摘要，释放 token 空间以继续对话。用户也可以手动触发压缩。

- 手动压缩：用户通过 `/compact` 指令触发，可附带自定义保留指令，指导压缩引擎重点关注哪些内容
- 自动压缩：每轮对话后检测 token 用量
  - 告警阶段：剩余空间低于告警阈值时，提示用户即将压缩。若后续剩余空间回升至告警阈值以上，告警自动取消
  - 压缩阶段：剩余空间低于压缩阈值时，自动执行压缩
  - 告警阈值大于压缩阈值（告警先于压缩触发），两个阈值均按上下文窗口百分比配置，每个 Agent 可独立设置
- 压缩只处理用户与 Agent 的对话消息，不触碰 system prompt，system prompt 内容完整保留
- 压缩结果为一条结构化摘要消息，覆盖六个维度：Goal / Constraints & Preferences / Progress / Key Decisions / Next Steps / Critical Context
- 连续压缩失败后自动进入保护暂停（仅阻止自动压缩再次触发，不影响活跃判定和归档），手动 `/compact` 成功后自动恢复
- 压缩前自动创建对话备份，压缩异常时可回滚至备份

> **交叉引用**：手动压缩由 `/compact` 指令触发，详见 [slash §F5](slash.md)（上下文压缩）。
> **交叉引用**：对话备份的通用机制详见 [F7](#f7-运行健康与安全)（运行健康与安全）。

### F4. 子 Session 委托与协调

Agent 可以将子任务委托给子 Session，并行委托多个子 Session，等待结果后继续决策。

- Agent 可以创建子 Session 来委托子任务，子 Session 执行完成后自动通知父 Session
- Agent 可以主动暂停当前对话以等待子 Session 完成后再恢复
- Agent 可以向已有未完成的子 Session 发送新任务
- Agent 可以终止子 Session，终止操作级联生效——终止某个子 Session 时，其所有后代一并终止
- 子 Session 完成后结果自动注入父 Session（带去重保护），Agent 不需要轮询
- 当前 Session 及所有子 Session 可被终止，级联生效
- 子 Session 超时通知：
  - 子 Session 运行时长达设定的超时上限时，系统向父 Session 注入超时通知。通知内容包含：设定的超时时间、实际运行时长、context window 使用情况及 token 用量
  - 若父 Agent 未终止该子 Session，系统在超时时间的 50% 间隔后再次注入通知，循环往复。间隔比例可配置
  - 父 Agent 收到通知后自行决定：终止子 Session、继续等待、或向用户汇报
  - 子 Session 的超时时间来源，按优先级：创建子 Session 时指定 > 目标 Agent 的独立配置 > Agent 全局默认值
- 暂停等待的解除条件：用户发送新消息、所有等待中的子 Session 正常完成、父 Agent 自行决定不再等待

> **交叉引用**：子 Session 结果注入的排队规则详见 [F9](#f9-消息注入)（消息注入）。
> **交叉引用**：`/stop` 指令触发 Session 终止，详见 [slash §F3](slash.md)（会话管理）。

### F5. LLM 交互控制

用户控制 LLM 调用的推理深度。Agent 的回复实时流式推送给用户。

- Session 维护当前生效的推理深度

> **交叉引用**：推理深度档位定义、默认值、优先级和模型能力降级策略详见 [llm §F4](llm.md)（推理强度控制）。
> **交叉引用**：运行时设置由 `/reasoning` 指令完成，详见 [slash §F10](slash.md)（推理深度控制）。
> **交叉引用**：流式输出的错误处理与不完整响应策略，详见 [llm §F2](llm.md)（流式实时输出）。
> **交叉引用**：Thinking 内容在流式输出与历史中的存储、可见性策略，详见 [llm §F3](llm.md)（推理过程可见与安全）。
> **交叉引用**：`/verbose` 指令控制信息展示等级，详见 [slash §F11](slash.md)（展示等级）。

### F6. 会话归档与清理

inactive 的 Session 自动归档，用户无需手动管理。用户可配置归档数据的自动清理时间，默认不自动删除。

- inactive 的 Session 自动归档：标记为归档状态，不再作为活跃 Session
- inactive 判定条件 = LLM推理为否 AND 同步工具等结果为否 AND 后台任务为否 AND 子Session为否 AND 距上次用户活动超过配置的 inactive 时长（每个 Agent 可独立配置，见本节后续说明）。四个活跃维度由 F11 定义
- 用户配置清理时间后，已归档超过该时长的 Session 彻底删除（元数据 + 对话记录）
- 每个 Agent 可独立配置 inactive 时长和清理时间，主 Session 与子 Session 可以分别设置
- 各配置项独立回退：未配置的项使用系统默认值（inactive 30 分钟归档、归档数据不自动删除）
- 系统对活跃 Session 和对话记录做双向一致性校验——有标记无对话视为损坏并清理，有对话无标记视为孤立并清理

### F7. 运行健康与安全

Agent 对话过程中，系统自动检测异常并提供保护机制，防止对话上下文损坏。

- 每轮对话结束后自动检测：响应超时、空响应、结构化异常等问题
- 可配置的自动质量检查：检测 Agent 是否陷入工具调用死循环
- 可配置的自动质量检查：检测 Agent 是否只计划不执行
- 对涉及对话历史完整性的操作（压缩、system prompt 修改），执行前自动创建对话备份，异常时可回滚到上一个安全状态
- 系统异常中断时，自动识别未完成的工具调用、子 Session 委托和未发送的出站消息。未发送的出站消息自动重投递；其余操作注入恢复通知，由 Agent 自行决定如何处理



### F8. 工作目录

用户可以设置文件操作的默认路径。

- 工作目录在 Session 创建时初始化为默认值，恢复时重置为默认值

> **交叉引用**：工作目录的查看与变更由 `/pwd`、`/cd`、`/git` 指令完成，详见 [slash §F7](slash.md)（工作目录操作）。

### F9. 消息注入

后台任务、记忆注入和子 Session 结果以消息形式注入消息队列，Agent 在后续轮次中按常规对话流程处理。

- 后台工具完成时，结果按优先级（now > next > later）注入消息队列
- 记忆注入通过 Session 实现，具体内容与注入位置由 memory 模块定义
- 记忆注入与后台消息注入互不冲突，可共存于同一批消息

> **交叉引用**：子 Session 结果的注入由 [F4](#f4-子-session-委托与协调)（子 Session 委托与协调）定义。
> **交叉引用**：记忆搜索结果的注入位置规则详见 [memory §F4](memory.md)（对话中自动注入相关记忆）。

### F10. 消息排队

用户消息按以下阻塞规则分派。判定条件使用 F11 的四维活跃维度。

排队条件 = LLM推理为是 OR 同步工具等结果为是。

- 满足排队条件时：用户消息按序排队，排队时提示用户「⏳ 正在排队…」
  - LLM 推理结束后注入排队消息
  - 同步工具结果返回后，工具结果与排队消息同批注入
- 不满足排队条件时：用户消息立即注入——无论后台任务和子 Session 是否活跃。Session 通过后台工具完成通知和子 Session 完成通知提醒 Agent 有待处理的后台任务，Agent 自行决定如何应对
- 非用户消息（子 Session 完成通知、后台工具结果、记忆注入等）与用户消息沿用同一阻塞框架：
  - 满足排队条件时：同批积压的非用户消息（按到达时间先后）优先注入，随后注入排队中的用户消息
  - 不满足排队条件时：非用户消息立即注入

> **交叉引用**：斜杠指令的排队/立即语义由 Gateway 路由决策决定，详见 [gateway §F5](gateway.md)（斜杠指令拦截与分派）。

### F11. Session 活跃维度

Session 在任意时刻可以同时处于多个活跃维度，每个维度独立开启或关闭。活跃维度由 Session 维护。

- LLM 正在推理或流式输出
- Agent 调用了工具并等待其返回结果以继续推理（同步调用）
- Agent 异步调用了工具（后台任务），不阻塞当前推理流程
- Session 有未完成的子 Session（已创建但未完成/未终止）

四个维度独立判定，不在此处定义复合状态。各功能模块按需从四维组合自己的判定条件：

- 消息分派：由 F10 定义判定条件
- 归档判定：由 F6 定义判定条件
- Workflow 验收：由 [workflow §F3](workflow.md)（步骤引导执行）定义判定条件
- 优雅关闭：由 [daemon §F2](daemon.md)（优雅关闭）定义判定条件

> **交叉引用**：Workflow 对 session 活跃维度的验收触发详见 [workflow §F3](workflow.md)（步骤引导执行）。

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

## 非功能需求

- **可靠性**：对话记录不能因系统重启或异常崩溃而丢失。正在执行的操作在崩溃后能被识别和通知
- **可恢复性**：系统重启后，所有活跃 session 应在 10 秒内完成扫描和恢复
- **性能**：Agent 的回复应在流式模式下实时逐字展示，首 token 延迟不受会话数量影响。后台维护任务（归档扫描）不应影响用户对话的响应延迟
- **可配置性**：每个 Agent 的 inactive 时长、清理时间可独立配置，主 Session 与子 Session 可以分别设置；各配置项独立回退到系统默认值
- **会话独立性**：会话路由、归档恢复等日常操作的响应速度不随用户历史会话总量增加而下降。系统重启恢复时间取决于当前活跃会话数，与历史归档会话总量无关
- **可观测性**：用户可以查看 LLM 用量统计和缓存利用情况，详见 [llm §F9](llm.md)（用量统计）
