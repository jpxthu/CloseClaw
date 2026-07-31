# Session 需求

## 概述

Session 模块为用户提供 Agent 对话上下文的持久化、可恢复与可管理能力——每次对话自动保存，系统重启不丢失历史，闲置自动归档。

## 功能需求

### F1. 对话持久化与恢复

用户与 Agent 的对话自动持久化，系统重启后 Agent 能接续之前的对话。已写入对话历史的消息完整保留。

- 用户发送消息时，系统自动查找该对话对应的 session——若存在则复用，若已归档则恢复，否则创建新 session
- 会话由平台、会话发起方、对端、账号四个维度共同标识。相同标识下可以有多个历史 session，当前活跃的是最后活跃时间最近的一个

> **交叉引用**：新 session 由 `/new` 指令触发创建，详见 [slash §F3](slash.md)。
- 归档的 session 被访问时自动恢复，恢复时用户收到「正在恢复会话…」提示，恢复后 Agent 的 system prompt 按最新配置重新注入
- 系统重启时，自动扫描所有活跃 session，对有未完成操作（工具调用、子 Session 委托、未发送的出站消息）的 session 注入恢复通知。重启前未发送的出站消息自动重投递


### F2. Agent 角色与能力配置

Session 重建时触发 Agent 的 system prompt 重新注入，注入内容反映当前最新的 bootstrap 文件、Skill 和工具定义。

> **交叉引用**：完整触发事件清单与缓存失效策略详见 [system_prompt §F6](system_prompt.md)（内容缓存与自动刷新）。
> **交叉引用**：技能清单的格式和热重载机制详见 [skills §F4](skills.md)（技能清单）、[skills §F5](skills.md)（热重载）。

- 用户追加的 system prompt 自定义指令持久化保存，归档恢复后完整保留，不参与对话压缩

> **交叉引用**：追加指令的交互方式（/system add/list/clear）由 [slash §F6](slash.md) 定义。

> **交叉引用**：
> - bootstrap 文件的清单与注入顺序，由 [system_prompt §F1](system_prompt.md) 定义
> - 子 session 的文件加载范围，由 [system_prompt §F8](system_prompt.md) 定义
> - 缓存失效与文件变更的自动反映，由 [system_prompt §F6](system_prompt.md) 定义

### F3. 长对话压缩

当对话历史接近上下文窗口上限时，系统自动将对话压缩为结构化摘要，释放 token 空间以继续对话。用户也可以手动触发。

- 手动压缩：用户可附带自定义保留指令，指导压缩引擎重点关注哪些内容

> **交叉引用**：手动压缩由 `/compact` 指令触发，详见 [slash §F5](slash.md)。
- 自动压缩：每轮对话后检测 token 用量
  - 预警阶段：剩余空间低于告警阈值时，提示用户即将压缩
  - 触发阶段：剩余空间低于压缩阈值时，自动执行压缩
- 压缩只处理用户与 Agent 的对话消息，不触碰 system prompt，其内容完整保留
- 压缩结果为一条结构化摘要消息，覆盖六个维度：Goal / Constraints & Preferences / Progress / Key Decisions / Next Steps / Critical Context
- 连续压缩失败后自动进入保护暂停（仅阻止自动压缩再次触发，不影响活跃判定和归档），手动 `/compact` 成功后自动恢复
- 压缩前自动创建对话备份，压缩异常时可回滚

### F4. 子 Session 委托与协调

Agent 可以将子任务委托给子 Session，并等待结果后继续决策。支持并行委托多个子 Session。

- Agent 可以创建子 Session 来委托子任务，子 Session 执行完成后自动通知父 Session
- Agent 可以暂停当前对话，等待所有子 Session 完成后再恢复
- Agent 可以向已有子 Session 发送新任务
- Agent 可以终止子 Session，级联终止该子 Session 及其所有后代
- 子 Session 完成后结果自动注入父 Session（带去重），Agent 不需要轮询
- 当前 Session 及所有子 Session 可被终止，级联生效

> **交叉引用**：`/stop` 指令触发 session 终止，详见 [slash §F3](slash.md)。
- 子 Session 超过预期时长时，系统向父 Session 注入超时预警通知；子 Session 继续执行，完成后正常回传结果
- 子 Session 超过超时上限时，系统终止该子 Session 并级联终止其所有后代，并通知父 Session
- 暂停等待有超时保护：系统取所有等待中子 Session 的超时上限最大值，加上缓冲（默认 1 分钟）作为最长等待时间
- 以下情况提前解除等待：用户发送新消息、所有子 Session 正常完成、子 Session 超时预警、子 Session 达到超时上限被终止
- 超过最长时间后等待自动解除，通知父 Agent 部分子 Session 可能超时，父 Agent 可自行决定继续等待或调查问题

### F5. LLM 交互控制

用户控制 LLM 调用的推理深度，Agent 的回复实时流式推送给用户。

- Session 维护当前生效的推理深度

> **交叉引用**：推理深度档位定义、默认值、优先级和模型能力降级策略详见 [llm §F4](llm.md)（推理强度控制）。运行时设置由 `/reasoning` 指令完成，详见 [slash §F10](slash.md)。
- 流式输出：Agent 回复实时渲染后逐步呈现给用户。流式响应出错时，已渲染给用户的内容保留可见，不完整回复不写入消息历史
- Thinking 内容默认不在流式输出中展示，但保留在消息历史中供后续轮次参考
- 用户可在流式输出中切换 Thinking 内容的显示或隐藏

> **交叉引用**：`/verbose` 指令控制信息展示等级，详见 [slash §F11](slash.md)。

### F6. 会话归档与清理

inactive 的会话自动归档，用户无需手动管理。用户可配置归档数据的自动清理周期，默认不自动删除。

- inactive 的 session 自动归档：标记为归档状态，不再作为活跃会话
- inactive 判定依据 session 活跃维度（详见 F11）：所有活跃维度均为 false，且距上次用户活动超过配置的 inactive 时长
- 用户配置清理时间后，已归档超过该时长的 session 彻底删除（元数据 + 对话记录）
- 每个 Agent 可独立配置 inactive 时长和清理时间，主 Session 与子 Session 可以分别设置
- 未配置时按默认配置（inactive 30 分钟归档、归档数据不自动删除）
- 系统对活跃 session 和文件系统做双向一致性校验——有元数据无对话记录视为损坏并清理，有对话记录无元数据视为孤儿文件并清理

### F7. 运行健康与安全

Agent 对话过程中，系统自动检测异常并提供保护机制，防止对话上下文损坏。

- 每轮对话结束后自动检测：响应超时、空响应、结构化异常等问题
- 可配置的自动质量检查，检测 Agent 是否只计划不执行、是否陷入工具调用死循环
- 执行对话历史的破坏性操作（压缩、system prompt 修改）前自动创建对话备份，异常时可回滚到上一个安全状态
- 系统异常中断时，自动识别未完成的工具调用、子 Session 创建和未发送的出站消息。未发送的出站消息自动重投递；其余操作注入恢复通知，由 Agent 决策如何处理（详见 F1）

### F8. 工作目录

用户可以设置文件操作的默认路径。

- 工作目录在 session 创建时初始化为默认值，恢复时重新初始化为默认值

> **交叉引用**：工作目录的查看与变更由 `/pwd`、`/cd`、`/git` 指令完成，详见 [slash §F7](slash.md)。

### F9. 消息注入

后台任务和记忆搜索结果以消息形式注入对话流，Agent 在后续轮次中按常规对话流程处理。

- 后台工具完成时，结果按优先级（now > next > later）注入消息队列
- 子 Session 完成时，结果注入父 Session 的消息队列，带去重保护
- 记忆注入经由 session 完成，具体内容与注入位置由 memory 模块定义

> **交叉引用**：记忆搜索结果的注入位置规则详见 [memory §F4](memory.md)
- 记忆注入与后台消息注入互不冲突，可共存于同一批消息

### F10. 消息排队

用户消息按以下阻塞规则分派。idle 的定义见 F11。

- Agent 正在推理或等待同步工具返回时：用户消息按序排队。LLM 推理结束后注入；同步工具结果返回后与用户消息一起注入
- Agent 有后台任务或有未完成的子 Session 时：用户消息立即注入——session 有其他机制（后台工具完成通知、子 Session 完成通知）提醒 Agent 还有后台任务待处理，Agent 自行判断如何应对
- 非用户消息（子 Session 完成通知、后台工具结果、记忆注入等）与用户消息遵循相同的阻塞规则：Agent 推理或等待同步工具时按序排队（排在用户消息前面），有后台任务或子 Session 时立即注入

> **交叉引用**：斜杠指令的排队/立即语义由 Gateway 路由决策决定，详见 [gateway §F5](gateway.md)。

### F11. Session 活跃维度

Session 在任意时刻可以同时处于多个活跃维度，每个维度独立开启或关闭。活跃维度由 session 维护。

- LLM 正在推理或流式输出
- Agent 调用了工具并等待其返回结果以继续推理（同步调用）
- Agent 异步调用了工具，不阻塞当前推理流程
- Session 有未完成的子 Session（已创建但未完成/未终止）

基于活跃维度，session 有以下复合状态：

- **idle**：前两项（LLM 推理和同步工具等待）均为 false——session 可以立即接收新用户消息，消息分派规则见 F10
- **inactive**：所有活跃维度均为 false，且距上次用户活动超过配置的 inactive 时长——触发归档判定，详见 F6

活跃维度由各消费方按需使用：

- 用户消息分派：idle 时直接分派；非 idle 时的阻塞规则见 F10
- 归档判定：inactive 时触发归档，详见 F6
- Workflow 验收：任一活跃维度为 true 时不注入验收清单

> **交叉引用**：Workflow 对 session 空闲状态的验收触发详见 [workflow §F3](workflow.md)。

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
- **可配置性**：每个 Agent 的 inactive 时长、归档清理周期可独立配置，主 Session 与子 Session 可以分别设置
- **会话独立性**：会话路由、归档恢复等日常操作的响应速度不随用户历史会话总量增加而下降。系统重启恢复时间取决于当前活跃会话数，与历史归档会话总量无关
- **可观测性**：用户可以查看跨轮次的 token 消耗统计和 LLM 缓存利用情况
