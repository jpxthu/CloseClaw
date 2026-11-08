# Agent 需求

## 概述

Agent 模块定义每个 AI Agent 的静态身份与配置档案组合，使 Owner 可以配置和查询差异化的 Agent。

Agent 是静态的配置身份；Session 是 Agent 的运行时实例。档案中各项能力的行为规则由对应功能模块定义，本文档定义 Agent 身份、档案的组合维度及两类 workspace 的归属。

## 功能需求

### F1. Agent 配置档案

每个 Agent 对应一份独立的配置档案。档案为纯静态定义，不包含运行时可变状态；Agent 的运行时会话由 Session 模块承载。

#### 能力维度

- **身份标识**：Agent 的唯一 ID 和显示名称
- **模型选择**：默认主模型与子 Session 默认模型
- **工具范围**：工具白名单与工具黑名单
- **技能范围**：技能白名单
- **子 Session 配置**：子 Session 的创建控制与超时配置
- **记忆配置**：可选的记忆模块参数

#### 能力规则

> **交叉引用**：身份人格与能力配置的区分。详见 [F2](#f2-身份与人格分离)（身份与人格分离）。
> **交叉引用**：模型配置与 Session 模型取值。详见 [llm §F11](llm.md)（Session 模型选择）。
> **交叉引用**：工具范围与默认工具。详见 [tools §F1](tools.md)（工具注册与发现）。
> **交叉引用**：技能范围与多 Agent 隔离。详见 [skills §F8](skills.md)（多 Agent 隔离）。
> **交叉引用**：子 Session 的创建控制与超时配置。详见 [session §F4](session.md)（子 Session 委托与协调）。
> **交叉引用**：记忆配置的覆盖语义。详见 [memory §F7](memory.md)（记忆挖掘的开关与参数配置）。

#### 配置及模块关联

> **交叉引用**：配置档案的目录布局与注册清单。详见 [config §F1](config.md)（多文件配置结构）。
> **交叉引用**：配置变更的检测与重载。详见 [config §F4](config.md)（配置重载）。
> **交叉引用**：入站消息与 Agent 的身份关联。详见 [im_adapter §F2](im_adapter.md)（入站消息归一化）。
> **交叉引用**：入站消息如何确定目标 Agent。详见 [gateway §F4](gateway.md)（普通消息路由到对话）。
> **交叉引用**：User 与 Agent 的权限身份。详见 [permission §F1](permission.md)（身份体系）。
> **交叉引用**：Session 与 Agent 的关联及档案变更的生效边界。详见 [session §F1](session.md)（对话持久化与恢复）。

### F2. 身份与人格分离

配置档案中的能力配置和身份人格是两层独立的概念：

- **配置档案**承载 F1 列出的各项能力维度（权限规则独立于配置档案，另行维护）
- **AGENTS.md** 承载 Agent 的身份人格与行为准则，不作为配置档案的字段或正文

> **交叉引用**：AGENTS.md 的固定位置、内容与加载。详见 [system_prompt §F1](system_prompt.md)（身份与行为准则定义）。
> **交叉引用**：AGENTS.md 的修改权限。详见 [permission §F4](permission.md)（Owner 特权）。
> **交叉引用**：两类 workspace 的归属。详见 [F7](#f7-两类-workspace)（两类 workspace）。
> **交叉引用**：权限规则与能力配置共同决定 Agent 能力边界。详见 [F3](#f3-agent-能力组合)（Agent 能力组合）。

### F3. Agent 能力组合

Agent 的能力边界由配置档案中的各项能力配置与权限规则共同决定，不依赖预定义的类型标签。Owner 通过组合各项配置提供差异化的 Agent；各能力维度的功能规则以 F1 引用的归属模块为准。

> **交叉引用**：权限规则与权限维度。详见 [permission §F2](permission.md)（权限维度）。

### F5. 初始 Agent 创建

首次使用系统时，Owner 在 CLI 配置向导的引导下创建一个初始 Agent。初始 Agent 的 ID 默认为 `master`，作为系统的基础入口。

> **交叉引用**：初始 Agent 的配置向导入口。详见 [cli §F4](cli.md)（配置向导与校验）。
> **交叉引用**：初始 Agent 的工具范围。详见 [tools §F1](tools.md)（工具注册与发现）。
> **交叉引用**：初始 Agent 的技能范围。详见 [skills §F8](skills.md)（多 Agent 隔离）。
> **交叉引用**：初始 Agent 的权限例外与沙盒模式。详见 [permission §F1](permission.md)（身份体系）。
> **交叉引用**：权限配置文件的不可读写约束。详见 [permission §F4](permission.md)（Owner 特权）。

### F6. 运行时配置查询

系统运行时支持按 Agent ID 查询 Agent 的完整配置档案。该查询为只读操作，返回该 Agent 在 F1 列出的全部静态能力维度，不包含运行时派生的能力（例如权限过滤后的实际工具清单）。

> **交叉引用**：配置文件组织与注册清单。详见 [config §F1](config.md)（多文件配置结构）。
> **交叉引用**：配置查询的 CLI 入口及访问限制。详见 [cli §F5](cli.md)（Agent 查询）。

### F7. 两类 workspace

每个 Agent 拥有两类 workspace，用途与归属不同：

- **Agent workspace**：该 Agent 专属，存放属于该 Agent 自身、跨 User 共用的文件；同一 Agent 的所有 User 共用一份，不能由任一 User 的 Agent-User workspace 代替。
- **Agent-User workspace**：每个 Agent-User 组合专属的独立目录，位于操作系统标准配置目录下，由系统自动创建和管理，与 Agent workspace 及同一 Agent 下其它 User 的目录相区分。

> **交叉引用**：两类 workspace 的授权与决策规则。详见 [permission §F3](permission.md)（权限决策模型）。
> **交叉引用**：两类 workspace 路径向 Session 的提供。详见 [system_prompt §F4](system_prompt.md)（运行时上下文注入）。
> **交叉引用**：操作系统标准配置目录的位置。详见 [platform §F2](platform.md)（操作系统标准配置目录）。

## 非功能需求

- **性能**：按 Agent ID 查询配置档案的时间复杂度为 O(1)，不随已注册 Agent 数量增长。
- **可靠性**：Agent ID 无法解析或 Agent 配置不可用时，系统向 User 给出错误提示。
- **可靠性**：并发创建 workspace 目录时不得相互干扰或失败。
- **安全性**：两类 workspace 的路径不得越出各自目录——Agent-User workspace 路径不得因 Agent ID 与 User ID 的特殊取值指向其它组合的目录，Agent workspace 共用文件也不得被误认成 Agent-User 专属文件。
- **可扩展性**：新增 Agent 能力维度时无需系统升级即可生效。
