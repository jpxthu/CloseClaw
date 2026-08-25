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

System Prompt 每次组装时，系统从技能注册中心读取当前可用技能，渲染技能清单并注入 System Prompt 固定位置。Agent 在每次 API 调用时通过 System Prompt 看到完整的可用技能列表。

清单中仅包含已声明 user-invocable 的技能（声明了 paths 的技能遵循 F6 条件激活规则，不在清单中）。

> **交叉引用**：System Prompt 组装触发时机见 [system_prompt §F6](system_prompt.md)（内容缓存与自动刷新）。System Prompt 各组成部分按固定顺序排列、配置不变时多次组装结果完全相同，详见 [system_prompt §F1](system_prompt.md)（身份与行为准则定义）。SP 组装结果在事件之间不变，确保模型服务端缓存持续命中，详见 [system_prompt §F7](system_prompt.md)（API 前缀缓存利用）。对话压缩不触碰 System Prompt，详见 [session §F3](session.md)（长对话压缩）。

清单按技能来源优先级排序（高优先级在前），同来源内按名称字母序排列。技能清单为空时不注入对应段落。

### F5. 技能文件变更

User 在 session 运行期间修改或新增 SKILL.md 文件后，技能变更不会在当前 session 自动生效。文件系统中的技能定义仅在下次 System Prompt 组装时反映。

> **交叉引用**：System Prompt 组装触发时机和数据源变更的生效规则见 [system_prompt §F6](system_prompt.md)（内容缓存与自动刷新）。

### F6. 条件激活

声明了 paths 字段的技能不在技能清单中（即使同时声明了 user-invocable）。当 Agent 操作的文件路径匹配某技能的 paths 模式时，该技能自动激活——系统为该技能在 session 内产生激活标记，并在下一个 turn 即时注入该技能的清单条目，Agent 无需等待 System Prompt 组装即可使用。

条件激活的注入条目与技能清单保持相同格式。仅注入清单条目（不含正文），正文在调用时按需加载（详见 F7）。激活标记的生命周期跟随当前 session，session 结束时清空。

> **交叉引用**：路径匹配检测和激活标记维护由 session 层完成，详见 [session §F2](session.md)（恢复时的 System Prompt 重建）。上下文压缩完成后 System Prompt 重新组装时，技能清单包含当前 session 已激活的条件技能，详见 [system_prompt §F6](system_prompt.md)（内容缓存与自动刷新）。

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

### F10. 技能创建

Agent 可通过内置技能获得创建技能文件的指导。User 在对话中描述需求后，Agent 按该技能的指令创建符合规范的 SKILL.md 文件，包含正确的 frontmatter 和指令正文。

## 关联设计文档

- [✓] skills/README.md
- [✓] skills/skill-definition.md
- [✓] skills/skill-listing-injection.md
- [✓] skills/skill-execution.md

## 非功能需求

- **加载效率**：技能目录扫描和清单注入不应对 User 感知的 session 启动速度产生明显影响
- **稳定性**：技能加载阶段的任何错误都不应导致 session 启动失败或进程崩溃
- **可观测性**：技能加载失败、同名冲突等异常情况应有明确提示，方便 User 定位问题原因
- **响应稳定性**：技能清单的注入和更新不得导致 Agent 对话质量下降或历史对话丢失
