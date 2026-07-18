# Skills 需求

## 概述

Skills 模块满足用户通过可复用技能插件扩展 Agent 能力的核心诉求——用户创建 SKILL.md 文件放入指定目录后，Agent 在下次 session 启动时自动发现并加载该技能，无需修改系统代码。

## 功能需求

### F1. 技能即插即用

User 将 SKILL.md 文件放入技能目录后，Agent 在下次 session 启动时自动发现并加载该技能。User 无需修改任何系统代码，也无需手动注册或重启 daemon 进程。

### F2. 技能目录层级

技能文件按作用域分层存放，多个层级的同名技能按固定优先级覆盖：

| 层级 | 作用域 | 优先级 |
|------|--------|--------|
| 项目级 | 仅当前项目 | 最高 |
| Agent 专属 | 仅该 Agent | 高 |
| 全局 | 所有 Agent | 中 |
| 外部复用 | 由 User 配置决定 | 低 |
| 内置 | 所有 Agent（系统默认提供） | 最低 |

User 通过在不同层级放置同名技能来实现覆盖——例如用项目级技能覆盖全局同名技能。外部复用层级允许 User 指定外部目录（如其他工具链的技能目录），直接复用其中的技能。

### F3. 技能配置

每个技能通过 SKILL.md 文件头部的 frontmatter 配置其行为，User 无需编写代码即可控制技能的各项属性：

- **description**（必填）：技能的简短描述，供 Agent 初步判断用途
- **when-to-use**：帮助 Agent 判断调用时机的提示
- **paths**：声明文件路径匹配模式，在 Agent 操作匹配文件时自动激活该技能
- **user-invocable**：控制该技能是否出现在技能清单中。默认不出现；声明后该技能出现在清单中（但声明了 paths 的技能遵循 F6 条件激活规则，不在初始清单中）。声明后 User 即可通过斜杠命令直接调用，无论是否出现在清单中
- **effort**：技能的成本估算，供 Agent 调度时参考

SKILL.md 正文（frontmatter 之后的指令文本）支持变量替换，User 可在正文中使用 `${SKILL_DIR}` 引用技能所在目录路径、使用 `${SESSION_ID}` 引用当前会话 ID。

> 技能仅提供纯 prompt 指令，不携带任何工具权限。
>
> **交叉引用**：工具权限由 Agent 配置统一管理。详见 [agent §F1](agent.md)（Agent 配置档案）、[permission §F2](permission.md)（权限维度）。

### F4. 技能清单

Session 启动时，系统向 Agent 注入一份技能清单（名称、描述、决策提示、成本估算），让 Agent 在对话开始时就知道有哪些技能可用。清单中仅包含已声明 user-invocable 的技能（声明了 paths 的技能的例外，见 F6）。

**增量注入**：后续 session turn 中若有新技能被激活或既有技能元数据变更（条件激活、热重载等），仅增量注入变更条目，不重发全量清单。增量注入须保持与初始注入相同的格式与位置，确保不破坏 Agent 对话上下文的连续性。
> **交叉引用**：技能清单由 session 模块作为 per-turn attachment 在每个 turn 注入 instruction block，不进入 system prompt 静态层。详见 [session §F2](session.md)（Agent 角色与能力配置）。

清单按技能来源优先级排序（高优先级在前），同来源内按名称字母序排列。技能清单为空时不注入。

> **交叉引用**：对话压缩时技能清单受保护（由 session 模块保证），详见 [session §F3](session.md)（长对话压缩）。

### F5. 热重载

User 在 session 运行期间修改或新增 SKILL.md 文件后，技能变更在下一个 session turn 以增量方式注入 Agent 的技能清单。User 无需手动操作也无需关注具体生效时机。
> **交叉引用**：生效时机由 session 生命周期驱动。文件监听触发的缓存失效和清单更新由 session 模块管理，详见 [session §F2](session.md)（Agent 角色与能力配置）。

### F6. 条件激活

声明了 paths 字段的技能不在初始技能清单中（即使同时声明了 user-invocable）。当 Agent 操作的文件路径匹配某技能的 paths 模式时，该技能自动激活——在下一个 session turn 以增量方式注入技能清单条目（不含正文，正文在调用时按需加载，详见 F7）。

条件激活的增量注入须保持与初始注入相同的格式与位置，确保不破坏 Agent 对话上下文的连续性。
> **交叉引用**：增量注入由 session 模块负责，路径匹配检测和激活标记维护在 session 层完成，详见 [session §F2](session.md)（Agent 角色与能力配置）。

### F7. 技能调用

Agent 在对话中根据技能的 description 和 when-to-use 判断是否调用某个技能。调用时系统加载技能正文并注入对话上下文，Agent 按技能指令继续执行。

User 也可通过斜杠命令直接调用声明了 user-invocable 的技能。

### F8. 多 Agent 隔离

多个 Agent 各自拥有独立的技能目录。Agent 专属目录下的技能仅对该 Agent 可见，不会影响其他 Agent。
> **交叉引用**：Agent 可用技能范围由 Agent 配置的白名单决定。详见 [agent §F1](agent.md)（Agent 配置档案）。

### F9. 错误容错

单个技能文件的错误不影响 session 正常运行：

- 技能目录路径不存在或无法访问时，跳过该来源，记录提示
- 单个 SKILL.md 格式错误或必填字段缺失时，跳过该技能，其他技能正常加载
- 同名冲突时，低优先级版本被跳过，记录提示

### F10. 技能创建工具

Agent 可通过内置的技能创建工具生成或修改技能文件。User 在对话中描述需求后，Agent 使用该工具创建符合规范的 SKILL.md 文件，包含正确的 frontmatter 配置和指令正文。

## 关联设计文档

- [✓] skills/README.md
- [✓] skills/skill-listing-injection.md

## 非功能需求

- **加载效率**：技能目录扫描和清单注入不应对 User 感知的 session 启动速度产生明显影响
- **稳定性**：技能加载阶段的任何错误都不应导致 session 启动失败或进程崩溃
- **可观测性**：技能加载失败、同名冲突等异常情况应有明确提示，方便 User 定位问题原因
- **响应稳定性**：技能清单的注入和更新不得导致 Agent 对话质量下降或历史对话丢失
