# Session 需求

## 概述

Session 模块为用户提供 Agent 对话上下文的持久化、可恢复与可管理能力——每次对话自动保存，系统重启不丢失历史，闲置自动清理。

## 功能需求

### F1. 对话持久化与恢复

用户与 Agent 的对话自动持久化，系统重启后 Agent 能接续之前的对话。每轮对话的历史消息完整保留，恢复到最近一次对话状态。

- 用户发送消息时，系统自动查找该对话对应的 session——若存在则复用，若已归档则恢复，否则创建新 session
- 会话路由键由 platform + sender_id + peer_id + account_id 组成。同一路由键下可以有多个历史 session（由 `/new` 创建），当前活跃的是最新（last_message_at 最大）的一个
- 归档的 session 被访问时自动恢复，恢复时用户收到「正在恢复会话…」提示，恢复后 Agent 看到的 system prompt 从最新配置文件重建
- 系统重启时，自动扫描所有活跃 session，对有未完成操作的 session 注入恢复通知。崩溃前未发送的出站消息自动重投递
- `/new` 指令在当前路由键下创建新 session，旧 session 保留但不再活跃

### F2. Agent 角色与能力配置

Agent 的行为规则和身份由 workspace 中的 bootstrap 文件（AGENTS.md、SOUL.md 等）定义。这些配置在每次 session 创建、归档恢复或对话压缩完成时自动注入 Agent 的 system prompt。

- Bootstrap 文件按固定顺序注入，其中 AGENTS.md（操作规程）排在最前
- 子 session 只加载基础角色文件（AGENTS.md、SOUL.md、IDENTITY.md、USER.md、TOOLS.md），不加载心跳工作流、自定义引导等主 session 专属文件
- Bootstrap 文件、Skill 文件和工具定义变更后，均在下次注入触发点（session 创建/恢复/压缩完成）自动使用最新内容
- 用户可以通过 `/system add <内容>` 在 system prompt 末尾追加自定义指令，`/system clear` 清除所有追加。追加内容持久化保存，归档恢复后完整保留，不参与对话压缩
- 工作目录（Agent 操作文件的默认路径）也在 system prompt 中自动体现

### F3. 长对话压缩

当对话历史接近上下文窗口上限时，系统自动将对话压缩为结构化摘要，释放 token 空间以继续对话。用户也可以手动触发。

- 手动压缩：`/compact` 指令，可附带自定义保留指令（如 `/compact 保留用户名和邮箱`）
- 自动压缩：每次对话后检测 token 用量
  - 预警阶段：剩余空间低于告警阈值时，提示用户即将压缩
  - 触发阶段：剩余空间低于压缩阈值时，自动执行压缩
- 压缩只处理对话消息（user/assistant），不触碰 system prompt（角色定义始终完整）
- 压缩结果为一条 boundary 消息，覆盖六个维度：Goal / Constraints & Preferences / Progress / Key Decisions / Next Steps / Critical Context
- 连续压缩失败后自动暂停（熔断），手动 `/compact` 成功后自动恢复
- 压缩前自动创建运行快照，压缩异常时可回滚

### F4. 子 Agent 委托与协调

Agent 可以将子任务委托给其他 Agent（子 session），并等待结果后继续决策。支持并行委托多个子 Agent。

- Agent 通过 sessions_spawn 创建子 session，子 Agent 执行完成后自动通知父 Agent
- Agent 通过 sessions_yield 主动暂停当前对话，等待所有子 Agent 完成后再恢复
- Agent 通过 sessions_steer 向已有子 session 发送新任务
- Agent 通过 sessions_kill 终止子 session（级联终止该子 session 及其所有后代）
- 子 Agent 完成后，结果通过消息队列注入父 Agent 的对话流。Agent 不需要轮询子 Agent 状态
- 用户可以通过 `/stop` 终止当前 session 及其所有子 session
- 子 Agent 在可配置时间内未完成时，系统解除等待状态并通知父 Agent 超时

### F5. LLM 交互控制

用户控制 LLM 调用的推理深度，Agent 的回复实时流式推送给用户。

- 推理深度支持四档（Low / Medium / High / Max）和 off 状态。`/reasoning <level|off>` 设置档位，`/reasoning`（无参）查询当前设置值与实际生效值（含自动降级后的结果）
- 模型不支持的推理档位自动降级到支持的最接近档位
- 流式输出：Agent 回复经出站管道实时渲染后逐步呈现给用户，Thinking 内容默认隐藏。流式响应中途出错时，已推送的部分保持不变，但不完整回复不会写入对话历史
- Agent 的 Thinking 内容默认对用户隐藏，但保留在消息历史中供后续对话参考
- 用户可以通过 `/verbose` 指令覆盖 Thinking 的显示行为

### F6. 会话归档与清理

闲置的会话自动归档，过期归档自动清理，用户无需手动管理。

- 超过配置空闲时间的 session 自动归档：标记 archived 状态，从活跃路由中移除
- 已归档超过配置清理时间的 session 彻底删除（元数据 + 对话记录）
- 每个 Agent 可独立配置空闲时间和清理时间，主 Agent 与子 Agent 分别设置
- 未配置时按硬编码兜底（空闲 30 分钟归档、清理永不过期）
- 归档前检查 session 是否有未完成操作，有则跳过本次归档
- 系统对活跃 session 和文件系统做双向一致性校验——有元数据无对话记录视为损坏并清理，有对话记录无元数据视为孤儿文件并清理

### F7. 运行健康与安全

Agent 对话过程中，系统自动检测异常并提供保护机制，防止对话上下文损坏。

- 每次对话轮次结束后自动检测：响应超时、空响应、结构化异常等问题
- 可配置可选的 Hook 审查（轻量 LLM 质量门禁），检测 Agent 是否只计划不执行、是否陷入工具调用死循环
- 对话历史发生破坏性操作（压缩、system prompt 修改）前自动创建快照，异常时可回滚到上一个安全状态
- 子 Agent 完成后，结果投递到父 Agent 的消息队列（崩溃场景下由恢复机制兜底）
- 系统崩溃时，自动识别未完成的工具调用、子 session 生成和出站消息，恢复时通知 Agent

### F8. 工作目录

用户可以设置文件操作的默认路径。

- `/pwd` 查看当前 session 的工作目录
- `/cd <路径>` 变更当前 session 的工作目录
- 工作目录在 session 创建时初始化为默认值，恢复时重新初始化
- `/git` 在工作目录上获取当前分支信息

### F9. 消息注入

后台任务和记忆搜索结果以消息形式注入对话流，Agent 在后续轮次中按常规对话流程处理。

- 后台工具完成时，结果按优先级（now > next > later）注入消息队列
- 子 Agent 完成时，结果注入父 Agent 的消息队列，带去重保护
- 记忆搜索结果（来自 memory 模块的 active-searcher）在每条消息前或后注入，提供相关历史上下文
- 记忆注入与后台消息注入互不冲突，可共存于同一批消息

### F10. 消息排队

当 Agent 正在执行操作时，任何来源的消息自动排队等待处理。

- Agent 忙碌时，所有消息进入 FIFO 等待队列，空闲后按序分派处理
- 即时指令（/stop、/status、/help 等）绕过排队队列，立即处理

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
- **可恢复性**：系统重启后，所有活跃 session 应在秒级完成扫描和恢复
- **性能**：Agent 的回复应在流式模式下实时逐字展示，首 token 延迟不受 session 管理开销影响。后台维护任务（归档清理）不应影响用户对话的响应延迟
- **可配置性**：每个 Agent 的会话空闲时间、归档清理周期可独立配置，主/子 Agent 分别设置
- **可观测性**：用户可以查看跨轮次的 token 消耗统计和缓存命中率
