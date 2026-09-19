# Session 需求

## 概述

Session 模块为 User 提供 Agent 对话上下文的持久化、可恢复与可管理能力——每次对话自动保存，系统重启不丢失历史，inactive 自动归档。

## 功能需求

### F1. 对话持久化与恢复

User 与 Agent 的对话自动持久化，已写入对话历史的消息完整保留。系统重启后 Agent 能接续之前的对话。

- 对话历史是 Session 的组成部分，与 Session 不可分离：Session 存在即包含其对话历史，不存在无归属的对话历史
- Session 分为**主 Agent Session**（由 User 消息直接触发创建，不由其他 Session 派生）与**子 Session**（由 Agent 委托创建、从属于某父 Session）；两者的生命周期判定框架一致（均按 F6 的四维 inactive 判据归档与清理）
- User 发送消息时，系统自动查找该对话对应的 Session——若存在未归档 Session 则复用其中最后活跃时间最新的一个；否则若存在归档中或已归档 Session，则恢复其中最后活跃时间最新的一个；否则创建新 Session
- 主 Agent Session 由入站消息的平台、发送者、会话对端、账号四项标识共同确定；同一组标识下可有多个历史 Session，当前使用的是最后活跃时间最新的一个。字段定义见 [im_adapter §F2](im_adapter.md)（入站消息归一化）
- 系统重启后，所有未归档 Session 自动恢复（整体恢复，非逐个查找）；子 Session（含持久子 Session）与主 Agent Session 一致恢复，不降级为一次性子 Session
- Session 在 Agent 范围内隔离：消息先路由到接收该消息的机器人所绑定的 Agent（见 [gateway §F4](gateway.md)（普通消息路由到对话）），再按上述四项标识查找 Session
- 处于归档中或已归档的 Session 被 User 通过消息访问时（按四项标识匹配）自动恢复：
  - 若 Session 归档中，等待归档完成后自动恢复；等待期间提示 User「会话归档中，稍后恢复…」
  - 若 Session 已归档，恢复时提示 User「正在恢复会话…」
  - 恢复后以最新内容重新注入 System Prompt，见 [system_prompt §F6](system_prompt.md)（内容缓存与自动刷新）

> **交叉引用**：强制开启新 Session 由 `/new` 指令触发（即使当前标识下已有未归档 Session）。详见 [slash §F3](slash.md)（Session 管理）。
> **交叉引用**：系统崩溃后的恢复流程详见 [F7](#f7-运行健康与安全)（运行健康与安全）。

### F2. 恢复时的追加指令保留

- System Prompt 追加指令持久化保存，归档恢复后完整保留

> **交叉引用**：Session 恢复时 System Prompt 重新注入的触发时机、完整触发事件清单与缓存失效策略详见 [system_prompt §F6](system_prompt.md)（内容缓存与自动刷新）。
> **交叉引用**：追加指令的交互方式（/system add/list/clear）详见 [slash §F6](slash.md)（System Prompt 追加）。

### F3. 长对话压缩

对话历史过长会挤占上下文窗口，系统通过压缩将其转为结构化摘要以释放 token 空间继续对话。压缩支持自动与手动两种触发方式。

- 手动压缩：User 可手动触发压缩，并可指定需要重点保留的内容。携带保留指示时，系统将其并入压缩调用，要求摘要重点保留指示所指内容（见下方 prompt 的「User-specified retention focus」一行）
- 自动压缩：每轮对话（一次 User 消息进入至 Agent 完成响应）结束后检测上下文剩余 token 空间
  - 告警阶段：剩余空间不高于告警阈值时，提示 User 即将压缩。若后续剩余空间回升至告警阈值以上，告警自动取消
  - 压缩阶段：剩余空间不高于压缩阈值时，自动执行压缩
  - 告警阈值与压缩阈值均以「剩余空间占上下文窗口的百分比」衡量，且满足 告警阈值 > 压缩阈值——数值越大代表剩余空间越充裕、越早触发，故告警先于压缩；每个 Agent 可独立设置，未配置时使用系统默认值（告警阈值默认 25%、压缩阈值默认 20%）
- 上下文溢出兜底：即使尚未达到压缩阈值，若 LLM 调用因超出上下文窗口而失败，系统同样触发一次自动压缩并重试该次调用（重试默认 1 次）
- 自动压缩（阈值触发）保留最近一段对话原文，仅将其余更早的对话压缩为摘要。保留区比例以「保留原文的 token 数占当前 Session 上下文窗口（完整窗口容量，不扣除 System Prompt 占用）的百分比」衡量（默认 16%、每个 Agent 可独立配置），保留方式为自最近消息向前累计至该预算；被保留的原文原样保留、不进入摘要。压缩不拆散工具调用与其结果
- 手动 `/compact` 与上下文溢出兜底触发的压缩不保留最近原文（保留区比例按 0% 处理，整段对话均进入摘要），以最大程度腾出空间
- 压缩对象为 User 与 Agent 的对话消息、工具调用与其结果，以及注入的非 User 消息；System Prompt 内容完整保留、不参与压缩
- 压缩在上下文视图与存储层两层的表现不同：上下文视图中，被压缩内容以一条结构化摘要原位替代，使发送给 LLM 的对话缩短；存储层中，压缩只追加、不销毁对话历史，被压缩的原文完整保留、仍可回溯
- 压缩过程中崩溃不影响已写入的对话历史（存储层只追加）；恢复时上下文视图由存储层重建——已压缩区间以对应摘要消息呈现，其余区间以原文呈现
- 压缩通过 LLM 生成摘要实现。压缩请求的构造为产品定义：
  - 输入 = 当前会话上下文（System Prompt 前缀 + 被压缩区间的对话消息），按原有角色与多轮顺序逐条原样发送，不合并成单条消息；工具调用与其结果成对发送，不拆散、不丢弃；被保留的最近原文不参与本次请求
  - 压缩指令（下列固定 prompt）作为**最后一条 user 消息**附在对话内容之后下发
  - 压缩请求复用会话既有的前缀缓存：与常规对话请求共享同一 System Prompt 前缀与被压缩消息前缀，仅追加末尾的指令消息，不因压缩重排或重新包装历史
  - 摘要以一条 user 角色的消息写回，内容以固定标记 `<compacted-summary>` … `</compacted-summary>` 包裹，以区别于真实 User 消息，并作为被压缩内容在上下文视图中的原位替代
  - 摘要模型默认为当前 Session 的模型；可配置为独立的摘要模型
- 压缩指令原文如下（随需求固定，属产品定义、开发不改写）：

```
You are compacting a conversation to free context space so the assistant can continue seamlessly.

Produce a summary with exactly these sections:
## Goal
## Constraints & Preferences
## Progress (Done / In Progress / Blocked)
## Key Decisions
## Next Steps
## Current Work
## Pending Jobs
## Critical Context

Rules:
- Preserve verbatim: file paths, commands, error strings, identifiers, numbers, and the user's corrections.
- Current Work must state precisely what was in progress just before compaction.
- Do not invent facts; omit unknowns.
- Reply with only the summary.

When the caller supplies a retention focus, it appears as the line below. Give those items priority and keep them verbatim; when none is supplied, the line reads none.
User-specified retention focus: <retention focus, or "none">
```
- 连续自动压缩失败（成功一次即清零重计）达到配置的熔断失败次数（默认 3 次）后自动进入保护暂停——保护暂停期间阈值触发与上下文溢出兜底均不再自动压缩（不影响活跃判定和归档），手动 `/compact` 仍可执行、成功后自动解除保护暂停；保护暂停期间若 LLM 调用因超出上下文窗口失败，该次调用按常规错误处理、不再自动压缩，User 可手动 `/compact` 恢复
- 「压缩失败」按压缩调用级判定：一次自动压缩（含上下文溢出兜底触发的自动压缩）未能成功产出一份可采用的摘要即计为一次失败——调用出错、超时，或产出的摘要不可用均计入；压缩相关配置（告警阈值 / 压缩阈值 / 保留区比例 / 熔断失败次数 / 摘要模型）变更后，自下一次上下文检测（阈值触发）起生效

> **交叉引用**：手动压缩由 `/compact` 指令触发。详见 [slash §F5](slash.md)（上下文压缩）。
> **交叉引用**：压缩完成后 System Prompt 重新组装详见 [system_prompt §F6](system_prompt.md)（内容缓存与自动刷新）。
> **交叉引用**：上下文窗口占用情况的展示详见 [llm §F9](llm.md)（用量统计）。
> **交叉引用**：压缩前自动备份的通用机制详见 [F7](#f7-运行健康与安全)（运行健康与安全）。

### F4. 子 Session 委托与协调

Agent 可以将子任务委托给子 Session（可并行委托多个），等待结果后继续决策。

- 子 Session 的任务描述注入到该子 Session 自身的 System Prompt 中，不属于对话消息，压缩时不受影响
- Agent 可以向已有子 Session 发送新任务——未完成的子 Session 与持久子 Session 均可接收（steer）
- Agent 可以终止子 Session，级联终止其所有后代（kill）。终止 = 停止该子 Session 正在执行的工具调用与 LLM 响应，是运行时动作，不是存储生命周期的一个状态；已被终止的子 Session 不再计入父 Session 的「子 Session 未完成」维度，其存储生命周期照常按 F6 的 inactive 判据推进
- 子 Session 的硬超时：运行时长达到该子 Session 的硬超时值时，系统自动终止该子 Session、级联终止其所有后代，并向父 Session 注入硬超时通知（父 Session 已归档则丢弃）。硬超时值不小于超时预警时间（否则预警不会触发）；硬超时的取值及优先级见 [agent §F7](agent.md)（子 Session 创建（Spawn））
- 持久子 Session：Agent 可以创建持久存活的子 Session，其生命周期与主 Agent Session 一致——同样按 F6 的四维 inactive 判据归档与清理，系统重启后按 F1 一致恢复，不降级为一次性子 Session（一次性子 Session：为单一任务创建、产出最后一条 assistant 消息后即结束的普通子 Session）
- 系统维护 Session 的父子关系，支持查询某 Session 的子 Session、完整子树与父 Session；销毁一个 Session 时，其尚未销毁的子 Session 一并销毁（级联清理）
- 子 Session 完成后，其结果以完成通知的形式通过消息队列注入父 Session（带去重保护）；该通知按 F10 的非 User 消息排队规则处理，Agent 不需要轮询
- 子 Session 超时预警通知：
  - 子 Session 的运行时长（自创建起计时）达到设定的超时预警时间时，系统向父 Session 注入超时预警通知。通知内容包含：设定的超时预警时间、已运行时长、子 Session 上下文窗口的 token 使用情况
  - 若父 Agent 未终止该子 Session，系统此后以「超时预警时间 × 间隔比例」为间隔再次注入通知，循环往复。间隔比例可配置（默认 50%）
  - 父 Agent 收到通知后自行决定：终止子 Session、继续等待，或向 User 汇报
  - 子 Session 超时预警时间的来源及优先级见 [agent §F7](agent.md)（子 Session 创建（Spawn））
- 父 Session 每轮对话开始时，系统注入当前未完成子 Session 摘要：未完成子 Session 数量及每个子 Session 的概要信息（子 Session 的 Agent 标识、任务简述、已运行时长）

> **交叉引用**：子 Session 的创建、超时取值优先级详见 [agent §F7](agent.md)（子 Session 创建（Spawn））。
> **交叉引用**：子 Session 创建控制（目标白名单、层级深度、并发数量）详见 [agent §F9](agent.md)（子 Session 创建控制）。
> **交叉引用**：持久子 Session 控制详见 [agent §F11](agent.md)（持久子 Session 控制）。
> **交叉引用**：父子关系追踪详见 [agent §F15](agent.md)（Spawn 层级追踪）。
> **交叉引用**：子 Session 管理工具（创建 / 发送任务 / 终止）的清单归属详见 [tools §F1](tools.md)（工具注册与发现）。
> **交叉引用**：子 Session 的权限沿链路收窄与拒绝行为详见 [permission §F9](permission.md)（子 Session 权限继承）。
> **交叉引用**：子 Session 相关各类注入通知（完成通知、超时预警通知、未完成子 Session 摘要）的排队规则详见 [F10](#f10-消息排队)（消息排队）。
> **交叉引用**：子 Session 完成通知的送达保证和僵死检测详见 [F7](#f7-运行健康与安全)（运行健康与安全）。
> **交叉引用**：`/stop` 终止当前 Session 运行、`/new` 创建新 Session（即使已有未归档 Session 也强制新建）。详见 [slash §F3](slash.md)（Session 管理）。

### F5. 推理强度控制

User 可控制 LLM 调用的推理强度；该设置在当前 Session 内持续生效，归档恢复后保留。

> **交叉引用**：推理强度档位定义、默认值、优先级和模型能力降级策略详见 [llm §F4](llm.md)（推理强度控制）。
> **交叉引用**：运行时设置由 `/reasoning` 指令完成。详见 [slash §F10](slash.md)（推理强度控制）。

### F6. Session 归档与清理

inactive 的 Session 自动归档，User 无需手动管理。User 可配置归档数据的自动清理时长，默认不自动删除。

- inactive 的 Session 自动归档：进入「归档中」，随即不再是未归档 Session；归档完成后标记为「已归档」
- inactive 判定条件：四维活跃维度均为否，且距该 Session 最后活跃时间超过配置的 inactive 时长，详见 [F11](#f11-session-活跃维度)（Session 活跃维度）
  - 「最后活跃时间」为该 Session 任一活跃维度最近一次为「是」的时刻；四维中任一维度为「是」期间该 Session 持续活跃，inactive 时长不推进、不触发判定
- User 配置清理时长后，已归档超过该时长的 Session 销毁（元数据 + 对话历史）
- 销毁仅由按时间执行的清理任务触发。归档不是销毁（归档 Session 可恢复），子 Session 自身完成后不触发销毁（结果注入父 Session 后保留、可供查询）
- 每个 Agent 可独立配置 inactive 时长和清理时长，主 Agent Session 与子 Session（含持久子 Session）可以分别设置
- 各配置项独立回退：未配置的项使用系统默认值（inactive 时长默认 30 分钟；清理时长默认未设置，即不自动删除）
- 新 Session 创建时使用当前配置；已在运行的 Session（含归档恢复后）沿用其创建时的 inactive 与清理配置，不随配置变更而变
- 归档与清理扫描参数（扫描间隔等）变更后自下一次扫描起生效
- 终止一个 Session（F4 的子 Session 终止、`/stop` 的当前 Session 终止）只停止其运行时活动，不改变上述归档与清理判定——生命周期仍完全由四维 inactive 判据与时间驱动
- 若父 Session 因系统异常（如崩溃）被归档（识别与恢复见 [F7](#f7-运行健康与安全)），系统记录告警日志，其未完成子 Session 的完成通知丢弃

> **交叉引用**：Session 配置的重载机制详见 [config §F4](config.md)（配置重载）。
> **交叉引用**：Session 生命周期参数的生效机制与归属详见 [config §F7](config.md)（生效机制与重启类判定）。
> **交叉引用**：Agent 配置与注册清单变更对 Session 的生效边界详见 [agent §F6](agent.md)（运行时配置查询）。
> **交叉引用**：后台任务输出文件随 Session 销毁回收详见 [tools §F5](tools.md)（后台任务执行）。
> **交叉引用**：媒体文件跟随 Session 生命周期的清理详见 [im_adapter §F9](im_adapter.md)（媒体文件收发与存储）。

### F7. 运行健康与安全

Agent 对话过程中，系统自动检测异常并提供保护机制，防止对话上下文丢失或损坏。

- 对话过程中自动检测：响应超时、空响应、结构化异常等问题
- 可配置的自动质量检查：检测 Agent 是否陷入工具调用死循环、是否只计划不执行
- 对涉及对话历史完整性的操作（压缩、System Prompt 修改），执行前自动创建对话备份，压缩或 System Prompt 修改失败时可回滚到上一个安全状态
- 系统崩溃时，自动识别未完成的工具调用、未完成的子 Session、未发送的出站消息。未发送的出站消息自动重投递；向未完成的工具调用与子 Session 原本所属的 Session 注入恢复通知，由 Agent 自行决定如何处理
- 系统定时扫描子 Session（F4 的超时预警通知不替代本项僵死兜底——父 Agent 选择继续等待时，仍受僵死判定保护）：
  - 已完成（四维活跃维度均为否，且该子 Session 的最后一条消息为 assistant 消息）但完成通知未成功送达父 Session 的，补推完成通知（若父 Session 已归档则丢弃）
  - 非已完成、四维活跃维度均为否且超过 5 分钟无新产出（无新 assistant 消息、该子 Session 的工具执行结果无变化）的，判定为僵死，自动终止（级联终止其所有后代，终止后不再视为未完成子 Session）并向父 Session 注入僵死通知（若父 Session 已归档则丢弃）

> **交叉引用**：后台任务与工具调用在 Session 停止或崩溃时的处理详见 [tools §F5](tools.md)（后台任务执行）。
> **交叉引用**：强制关闭遗留的未完成操作由下次启动的恢复流程处理详见 [daemon §F3](daemon.md)（强制关闭）。

### F8. 工作目录

每个 Session 拥有独立的工作目录，作为该 Session 文件操作的默认路径；在 Session 创建时初始化，归档恢复时重新确定。

- 工作目录的取值由下方解析顺序确定：创建时确定初值，恢复时按同一顺序重新确定（spawn 参数不可得时跳过该项）
- 工作目录的解析顺序：spawn 参数指定 > 目标 Agent 配置 > 系统默认工作目录

> **交叉引用**：工作目录的强制授权机制（不受任何 Deny 规则影响）详见 [permission §F3](permission.md)（权限决策模型）。
> **交叉引用**：工作目录作为 Agent 配置档案字段详见 [agent §F1](agent.md)（Agent 配置档案）。
> **交叉引用**：spawn 时的工作目录覆盖详见 [agent §F7](agent.md)（子 Session 创建（Spawn））。
> **交叉引用**：工作目录的查看与变更由 `/pwd`、`/cd`、`/git` 指令完成。详见 [slash §F7](slash.md)（工作目录操作）。

### F9. 消息注入

后台任务结果、记忆内容和子 Session 完成通知通过消息队列注入，Agent 在后续轮次中按常规对话流程处理。

- 后台任务完成时，结果携带优先级（now > next > later），按 [F10](#f10-消息排队)（消息排队）的排队规则注入消息队列
- 记忆注入与子 Session 完成通知不携带优先级
- 记忆注入通过 Session 的消息队列完成，具体内容与注入位置由 memory 模块定义
- 记忆注入与后台任务结果注入互不冲突，可共存于同一批消息

> **交叉引用**：子 Session 完成通知的注入由 [F4](#f4-子-session-委托与协调)（子 Session 委托与协调）定义。
> **交叉引用**：记忆搜索结果的注入位置规则详见 [memory §F4](memory.md)（对话中自动注入相关记忆）。

### F10. 消息排队

User 与非 User 消息按以下排队规则注入。排队条件使用 F11 的活跃维度：F11「推理中」为是，或「同步工具等待」为是。

- 满足排队条件时：User 消息按序排队，排队时提示 User「⏳ 正在排队…」
  - 因「推理中」排队的消息，在 LLM 推理结束后注入
  - 因「同步工具等待」排队的消息，在同步工具结果返回后与工具结果同批注入
- 不满足排队条件时：User 消息立即注入——无论后台任务和子 Session 是否活跃。Session 通过后台任务完成通知和子 Session 完成通知提醒 Agent 有待处理的后台任务结果或子 Session 结果，Agent 自行决定如何应对
- 非 User 消息（子 Session 完成通知、后台任务结果、记忆注入等）与 User 消息采用同一排队条件与规则：
  - 满足排队条件时：同批积压的非 User 消息按优先级（now > next > later）排序注入，同一优先级内按到达时间先后；未携带优先级的非 User 消息排在带优先级的非 User 消息之后，彼此按到达时间先后。随后注入排队中的 User 消息
  - 不满足排队条件时：非 User 消息立即注入

> **交叉引用**：斜杠指令的排队/立即语义由 gateway 路由决策决定。详见 [gateway §F5](gateway.md)（斜杠指令拦截与分派）。

### F11. Session 活跃维度

Session 在任意时刻可以在多个活跃维度上同时开启，每个维度独立开启或关闭。

- **推理中**：LLM 正在推理（含流式输出）
- **同步工具等待**：Agent 调用了工具并等待其返回结果以继续推理（同步调用）
- **后台任务**：Agent 异步调用了工具，不阻塞当前推理流程；该维度在有正在运行的后台任务期间为是，全部到达终态后为否
- **子 Session 未完成**：Session 有尚未完成的子 Session（含各级后代；已创建、尚未产生最后一条 assistant 消息或仍有活跃维度；已终止的子 Session 不再计入）。完成判据见 [F7](#f7-运行健康与安全)（运行健康与安全）

四维活跃维度的复合判定由各功能域按需组合：

- 消息排队：由 F10 定义排队条件
- 归档判定：由 F6 定义判定条件
- 运行健康判定（子 Session 完成与僵死）：由 F7 定义判定条件
- workflow 验收：由 [workflow §F3](workflow.md)（步骤引导执行）定义判定条件
- 优雅关闭：由 [daemon §F2](daemon.md)（优雅关闭）定义判定条件
- 斜杠指令路由：由 [gateway §F5](gateway.md)（斜杠指令拦截与分派）定义判定条件

### F12. 调试日志

Session 模块在以下环节记录调试日志：
- Session 创建、查找、归档恢复
- 对话历史的追加与修改
- 长对话压缩事件（触发原因、压缩结果概要，以及保护暂停的进入与解除）
- 活跃维度变化（任一维度开启或关闭）
- 子 Session 创建与完成
- 消息注入事件（后台任务结果、记忆注入、子 Session 完成通知）
- 运行健康与异常检测结果

> **交叉引用**：完整消息链路追踪详见 [debug_log §F1](debug_log.md)（完整消息链路追踪）。
> **交叉引用**：分层日志级别详见 [debug_log §F2](debug_log.md)（分层日志级别）。
> **交叉引用**：日志存储与保留详见 [debug_log §F3](debug_log.md)（日志存储与保留）。
> **交叉引用**：隐私保护详见 [debug_log §F4](debug_log.md)（隐私保护）。

## 非功能需求

> 复杂度符号：A = 当前未归档 Session 数量，N = 历史 Session 总量（含已归档）。

- **可靠性**：对话历史在系统重启或进程异常退出后完整保留，已写入历史的消息不丢失；崩溃时的操作识别与恢复详见 [F7](#f7-运行健康与安全)（运行健康与安全）
- **可恢复性**：系统重启后自动恢复所有未归档 Session。恢复耗时复杂度 O(A)，与 N 无关
- **性能**：Agent 回复实时展示（展示的片段粒度见 [im_adapter §F4](im_adapter.md)（流式增量渲染））。后台维护任务（归档扫描）不阻塞 User 对话的响应
- **安全性**：对话历史按 Session 隔离，仅该 Session 所属 User 与其绑定的 Agent 可访问，跨 User / 跨 Agent 不可读；Session 销毁后对话历史不可恢复
- **可配置性**：inactive 时长、清理时长按 Agent 独立配置，且主 Agent Session 与子 Session 可分别设置；压缩的告警阈值 / 压缩阈值 / 保留区比例 / 熔断失败次数 / 摘要模型、子 Session 超时预警时间 / 间隔比例按 Agent 独立配置；各配置项独立回退到系统默认值。生效时机：inactive 与清理配置对已有 Session 粘性保留（见 F6），压缩相关配置自下一次上下文检测起生效（见 F3）。配置重载机制详见 [config §F4](config.md)（配置重载）
- **可观测性**：Session 的关键操作（创建 / 查找 / 归档恢复、对话历史追加与修改、压缩、消息注入、活跃维度变化、子 Session 创建与完成、健康检测）均记录调试日志（见 [F12](#f12-调试日志)（调试日志）），可据以排查问题
- **Session 独立性**：Session 路由、委托、归档恢复等日常操作对 N 的复杂度为 O(1) 或 O(log N)，不对全量历史 Session 做 O(N) 遍历
- **长期运行稳定性**：系统累计委托大量子任务后，委托新子任务和 User 对话的响应速度不随已完成子任务数量增加而退化。已完成子任务的结果在其 Session 销毁前仍可查，但不持续占用运行资源；内存占用 O(A)，对 N 为 O(1)
