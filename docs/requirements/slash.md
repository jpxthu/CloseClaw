# Slash 需求

## 概述

Owner 和 User 可以通过以 `/` 开头的消息发送系统控制指令，这些指令不进入 LLM 对话流程，由 Gateway 拦截并分派给对应的指令处理器执行。

Slash 模块仅提供指令入口与分派机制；跨模块指令的业务功能由对应功能模块定义，本文档以交叉引用指向权威定义，不重复定义。

## 功能需求

### F1. 斜杠指令入口

Owner 和 User 发送以 `/` 开头的消息时，消息不进入 LLM 对话流程，而是作为斜杠指令被处理。无匹配处理器的指令向调用者返回友好错误提示，引导使用 `/help` 查看可用指令。

各指令的 Immediate 标记在后续各功能域中标注；未标注 Immediate 的指令默认为非 Immediate。

> **交叉引用**：斜杠指令的拦截、分派与 Immediate 排队语义由 Gateway 负责。详见 [gateway §F5](gateway.md)（斜杠指令拦截与分派）。
> **交叉引用**：排队条件定义详见 [session §F10](session.md)（消息排队）。
> **交叉引用**：Session 活跃维度详见 [session §F11](session.md)（Session 活跃维度）。
> **交叉引用**：不以 `/` 开头的普通消息路由详见 [gateway §F4](gateway.md)（普通消息路由到对话）。

### F3. Session 管理

Owner 和 User 可以创建新 Session，以及终止当前 Session 运行。

**指令**：
- `/new`：创建新 Session
- `/stop`：终止当前 Session 运行（Immediate）

> **交叉引用**：Session 创建与恢复详见 [session §F1](session.md)（对话持久化与恢复）。
> **交叉引用**：子 Session 终止详见 [session §F4](session.md)（子 Session 委托与协调）。

### F4. 状态查询

Owner 和 User 可以随时查看当前 Session 的运行状态。

**指令**：
- `/status`：查询 Session 状态（Immediate）

查询内容：模型、推理强度、上下文用量、缓存命中率、token 累计量、活跃子 Session 数。System Prompt 追加指令列表仅 Owner 可见，非 Owner User 的查询结果不包含该项。

> **交叉引用**：当前模型见 [llm §F1](llm.md)（多供应商统一对话）。
> **交叉引用**：推理强度见 [llm §F4](llm.md)（推理强度控制）。
> **交叉引用**：上下文用量、缓存命中率与 token 累计量见 [llm §F9](llm.md)（用量统计）。
> **交叉引用**：子 Session 见 [session §F4](session.md)（子 Session 委托与协调）。
> **交叉引用**：追加指令见 [system_prompt §F5](system_prompt.md)（追加指令管理）。

### F5. 上下文压缩

Owner 和 User 可以手动触发对话历史的上下文压缩。

**指令**：
- `/compact`：默认压缩
- `/compact <保留指示>`：携带保留指示的压缩

> **交叉引用**：压缩引擎的压缩范围和行为定义在 [session §F3](session.md)（长对话压缩）。

### F6. System Prompt 追加

Owner 可以在运行时向 System Prompt 的追加区动态添加指令，无需修改配置文件。多次追加的内容叠加保留，不覆盖已有追加。该功能仅 Owner 可用；非 Owner User 调用 `/system` 及其子指令时返回权限不足提示。

**指令**：
- `/system add <内容>`：追加一条指令
- `/system` 或 `/system list`：列出当前所有追加指令
- `/system clear`：清除全部追加指令

追加内容超过 500 字符时，直接拒绝并向 Owner 返回错误提示，不进行截断。`/system add` 不带内容时，向 Owner 返回用法提示。

> **交叉引用**：追加区在 System Prompt 中的位置由 [system_prompt §F5](system_prompt.md)（追加指令管理）定义。
> **交叉引用**：追加内容的存储和持久化由 [session §F2](session.md)（恢复时的追加指令保留）定义。

### F7. Git 操作

Owner 和 User 可以执行 Git 命令。Git 命令在当前 Session 所属 Agent-User 组合的 workspace 内执行（见 [permission §F3](permission.md)（权限决策模型））。

**指令**：
- `/git <参数>`：执行 Git 命令
  - 只读子命令（status、log、diff、show、branch（仅列出分支））直接执行，不进入权限评估
  - 写操作的评估与审批见 [permission §F2](permission.md)（权限维度）、[permission §F5](permission.md)（审批工作流）

> **交叉引用**：Owner 的命令执行豁免详见 [permission §F4](permission.md)（Owner 特权）。
> **交叉引用**：Git 写操作的权限评估见 [permission §F3](permission.md)（权限决策模型）。
> **交叉引用**：命令维度的黑名单、白名单与执行位置见 [permission §F2](permission.md)（权限维度）。

### F8. 命令执行

Owner 可以执行 Shell 命令。命令执行不向 User 开放——它不是 User 维度的权限，User 调用 `/exec` 收到权限不足提示。

**指令**：
- `/exec <命令>`：在当前 Session 所属 Agent-User 组合的 workspace 内执行 Shell 命令

> **交叉引用**：Owner 的命令执行豁免详见 [permission §F4](permission.md)（Owner 特权）。
> **交叉引用**：命令的黑名单、白名单与执行位置详见 [permission §F2](permission.md)（权限维度）、[permission §F12](permission.md)（沙盒）。
> **交叉引用**：权限规则的修改方式见 [permission §F6](permission.md)（权限配置管理）。

### F9. 帮助

Owner 和 User 可以查看所有当前可用的斜杠指令及其说明。帮助内容从系统当前支持的指令自动生成，系统升级引入的新指令自动出现在帮助中，无需单独维护帮助文案。

**指令**：
- `/help`：查看帮助（Immediate）

### F10. 推理强度控制

Owner 和 User 可以查询和设置当前 Session 的 LLM 推理强度。

**指令**：
- `/reasoning`（无参数）：查询当前推理强度档位（Immediate）
- `/reasoning low|medium|high|max|off`：设置推理强度档位（Immediate）

> **交叉引用**：推理强度的等级定义、默认值、优先级、降级策略及 `off` 的行为见 [llm §F4](llm.md)（推理强度控制）。

### F11. 展示等级

Owner 和 User 可以查询和设置当前 Session 的展示等级。设置等级不影响当前正在输出的消息，仅对后续新消息生效。

**指令**：
- `/verbose`（无参数）：查询当前展示等级（Immediate）
- `/verbose full|normal|off`：设置展示等级（Immediate）

> **交叉引用**：各展示等级对应的回复内容见 [processor_chain §F4](processor_chain.md)（出站回复展示等级控制）。

### F13. 审批指令

Owner 通过 `/approve-once`、`/approve-whitelist`、`/deny` 审批 Agent 操作。三条指令均为 Immediate，仅 Owner 可用。

**指令**：
- `/approve-once <单号> [agent|user|both]`：单次放行
- `/approve-whitelist <单号> [agent|user|both]`：加入白名单
- `/deny <单号>`：拒绝

维度参数指定放行作用在 Agent 维度、User 维度还是两者，未指定时按 `both` 处理；命令类操作固定为 `agent`（语义见 [permission §F5](permission.md)（审批工作流））。

> **交叉引用**：审批决策的完整语义（单次放行/加入白名单/拒绝）、审批单号的全局唯一性与命中规则、投递目标与超时时长详见 [permission §F5](permission.md)（审批工作流）。
> **交叉引用**：非 Owner 调用审批指令的权限不足提示详见 [gateway §F5](gateway.md)（斜杠指令拦截与分派）。

### F14. 调试日志

Slash 模块在以下环节记录调试日志：
- 指令匹配与分派结果
- 指令执行起止与耗时
- 指令执行异常

> **交叉引用**：日志框架定义详见 [debug_log §F1](debug_log.md)（完整消息链路追踪）、[debug_log §F2](debug_log.md)（分层日志级别）、[debug_log §F3](debug_log.md)（日志存储与保留）、[debug_log §F4](debug_log.md)（隐私保护）。

### F15. workflow 指令

Owner 通过 `/workflow <名称>` 启动对应的 workflow，仅 Owner 可用。`/workflow` 为非 Immediate 指令——满足排队条件时，在当前 LLM 调用结束后执行；启动语义由 workflow 模块定义。

> **交叉引用**：非 Owner 调用 `/workflow` 的权限不足提示详见 [gateway §F5](gateway.md)（斜杠指令拦截与分派）。

> **交叉引用**：启动语义与 workflow 模式进入详见 [workflow §F2](workflow.md)（workflow 启动）。
> **交叉引用**：workflow 模式退出（生命周期与清理）详见 [workflow §F8](workflow.md)（流程生命周期）。

### F16. 权限配置管理

Owner 可以通过斜杠指令查看、新增与删除权限规则，并设置沙盒模式；非 Owner 调用时收到权限不足提示。

**指令**：
- `/perm list`：列出当前生效的权限规则，按序编号
- `/perm allow <主体> <维度> <生效对象>`：新增一条允许规则
- `/perm deny <主体> <维度> <生效对象>`：新增一条拒绝规则
- `/perm remove <序号>`：删除 `/perm list` 中对应序号的规则
- `/perm sandbox <默认|严格|放行>`：设置当前会话的目标 Agent 的沙盒模式（见 [permission §F12](permission.md)（沙盒））

主体写法为 `agent:<Agent ID>` 或 `user:<User ID>`；维度取 [permission §F2](permission.md)（权限维度）定义的七个维度之一。规则主体与生效对象的约束、不细分对象的维度、高风险对象的限制详见 [permission §F6](permission.md)（权限配置管理）。

> **交叉引用**：规则要素、修改时的 Owner 校验与生效机制详见 [permission §F6](permission.md)（权限配置管理）。
> **交叉引用**：非 Owner 调用权限指令的权限不足提示详见 [gateway §F5](gateway.md)（斜杠指令拦截与分派）。

## 非功能需求

- Immediate 指令在 LLM 运行中必须可达，调用方不感知延迟
- 全部 Immediate 指令：
  - `/stop`
  - `/status`
  - `/reasoning`（各形态）
  - `/verbose`（各形态）
  - `/help`
  - 审批指令：`/approve-once`、`/approve-whitelist`、`/deny`

> **交叉引用**：审批指令（/approve-once、/approve-whitelist、/deny）的 Immediate 可达性由 Gateway 层保证。详见 [gateway §F5](gateway.md)（斜杠指令拦截与分派）。
