# Slash 需求

## 概述

Owner 和 User 可以通过以 `/` 开头的消息发送系统控制指令，这些指令不进入 LLM 对话流程，由 Gateway 拦截并分派给对应的指令处理器执行。

Slash 模块仅提供指令入口与分派机制；跨模块指令的业务功能由对应功能模块定义，本文档以交叉引用指向权威定义，不重复定义。

排队条件定义详见 [session §F10](session.md)（消息排队），Session 忙碌与空闲的判定详见 [session §F11](session.md)（Session 活跃维度），Immediate 指令的排队绕过语义详见 [gateway §F5](gateway.md)（斜杠指令拦截与分派）。

## 功能需求

### F1. 斜杠指令入口

Owner 或 User 发送以 `/` 开头的消息时，消息不被路由到 LLM 对话流程，而是由 Gateway 拦截后解析为指令名和参数，分派给对应的指令处理器执行。无匹配处理器的指令向 User 返回友好错误提示，引导使用 `/help` 查看可用指令。

各指令的 Immediate 标记在后续各功能域中分别标注；未标注 Immediate 的指令默认为非 Immediate。

> **交叉引用**：斜杠指令的拦截、分派与 Immediate 排队语义由 Gateway 负责，详见 [gateway §F5](gateway.md)（斜杠指令拦截与分派）。
> **交叉引用**：不以 `/` 开头的普通消息路由详见 [gateway §F4](gateway.md)（普通消息路由到对话）。

### F2. 模式切换

模式切换指令（`/plan`、`/mode`、`/execute`）的完整语法、参数和业务行为由 Mode 模块定义。Slash 模块仅提供 Gateway 层的指令拦截和分派机制（见 F1）。`/plan` 与 `/execute` 为非 Immediate 指令——Session 忙碌时在当前 LLM 调用结束后执行。`/mode` 在无参数查询形态下为 Immediate，在带参数切换形态下为非 Immediate。

> **交叉引用**：`/plan` 和 `/mode` 的语法和语义详见 [mode §F14](mode.md)（模式切换指令）。
> **交叉引用**：`/execute` 的触发语义详见 [mode §F4](mode.md)（执行触发）。
> **交叉引用**：模式类型定义和 Agent 行为约束详见 [mode §F1](mode.md)（运行模式）。

### F3. 会话管理

Owner 和 User 可以创建新会话，以及终止当前会话运行。

**指令**：
- `/new`：创建新会话
- `/stop`：终止当前会话运行（Immediate）

> **交叉引用**：会话创建与恢复详见 [session §F1](session.md)（对话持久化与恢复）；会话归档详见 [session §F6](session.md)（会话归档与清理）；子 Session 终止详见 [session §F4](session.md)（子 Session 委托与协调）。

### F4. 状态查询

Owner 和 User 可以随时查看当前会话的运行状态。

**指令**：
- `/status`：查询会话状态（模式、模型、推理强度、上下文用量、缓存命中率、token 累计量、活跃子 Session 数、工作目录、system prompt 追加指令列表）（Immediate）

> **交叉引用**：模式见 [mode §F1](mode.md)（运行模式）。
> **交叉引用**：当前模型见 [llm §F1](llm.md)（多供应商统一对话）。
> **交叉引用**：推理强度见 [llm §F4](llm.md)（推理强度控制）。
> **交叉引用**：上下文用量、缓存命中率与 token 累计量见 [llm §F9](llm.md)（用量统计）。
> **交叉引用**：子 Session 见 [session §F4](session.md)（子 Session 委托与协调）。
> **交叉引用**：工作目录见 [session §F8](session.md)（工作目录）。
> **交叉引用**：追加指令见 [system_prompt §F5](system_prompt.md)（动态指令管理）。

### F5. 上下文压缩

Owner 和 User 可以手动触发对话历史的上下文压缩。

**指令**：
- `/compact`：默认压缩
- `/compact <保留指示>`：携带保留指示的压缩

> **交叉引用**：压缩引擎的压缩范围和行为定义在 [session §F3](session.md)（长对话压缩）。

### F6. system prompt 追加

Owner 和 User 可以在运行时向 system prompt 的追加区动态添加指令，无需修改配置文件。多次追加的内容叠加保留，不覆盖已有追加。

**指令**：
- `/system add <内容>`：追加一条指令
- `/system` 或 `/system list`：列出当前所有追加指令
- `/system clear`：清除全部追加指令

追加内容超过 500 字符时，直接拒绝并向 User 返回错误提示，不进行截断。`/system add` 不带内容时，向 User 返回用法提示。

> **交叉引用**：追加区在 system prompt 中的位置由 [system_prompt §F5](system_prompt.md)（动态指令管理）定义。
> **交叉引用**：追加内容的存储和持久化由 [session §F2](session.md)（恢复时的 System Prompt 重建）定义。

### F7. 工作目录操作

Owner 和 User 可以变更和查看当前会话的工作目录，以及执行 Git 命令。

**指令**：
- `/cd <路径>`：变更工作目录，切换前校验路径是否存在，路径不存在时提示错误；切换成功后回复路径及 Git 分支信息
- `/pwd`：查看当前工作目录
- `/git <参数>`：执行 Git 命令。只读子命令（status、log、diff、show、branch（仅列出分支））无需权限审批直接执行，写操作必须经权限审批，审批不可绕过

> **交叉引用**：工作目录的定义（字段、默认值、变更后的状态展示、Git 命令输出）见 [session §F8](session.md)（工作目录）。
> **交叉引用**：Git 写操作的权限审批见 [permission §F3](permission.md)（权限决策模型）。

### F8. 命令执行

Owner 可以执行任意 Shell 命令，执行前必须经权限审批，审批不可绕过；Owner 的命令在审批中默认放行。

**指令**：
- `/exec <命令>`：执行 Shell 命令

> **交叉引用**：命令执行的权限评估由 Permission 模块负责，详见 [permission §F3](permission.md)（权限决策模型）；User 默认权限与授权方式详见 [permission §F1](permission.md)（身份体系）、[permission §F6](permission.md)（权限配置管理）。

### F9. 帮助

Owner 和 User 可以查看所有当前可用的斜杠指令及其说明。帮助内容从系统当前支持的指令自动生成，系统升级引入的新指令自动出现在帮助中，无需单独维护帮助文案。

**指令**：
- `/help`：查看帮助（Immediate）

### F10. 推理强度控制

Owner 和 User 可以查询和设置当前会话的 LLM 推理强度。

**指令**：
- `/reasoning`（无参数）：查询当前推理强度档位（Immediate）
- `/reasoning low|medium|high|max|off`：设置推理强度档位（Immediate）

> **交叉引用**：推理强度的等级定义、默认值、优先级、降级策略及 `off` 的行为见 [llm §F4](llm.md)（推理强度控制）。

### F11. 展示等级

Owner 和 User 可以查询和设置当前会话的展示等级。设置等级不影响当前正在输出的消息，仅对后续新消息生效。

**指令**：
- `/verbose`（无参数）：查询当前展示等级（Immediate）
- `/verbose full|normal|off`：设置展示等级（Immediate）

> **交叉引用**：展示等级的过滤内容定义见 [processor_chain §F4](processor_chain.md)（出站回复冗余控制）。

### F12. plan 浏览

`/plans` 指令的完整语法、参数和业务行为由 Mode 模块定义。Slash 模块仅提供 Gateway 层的指令拦截和分派机制（见 F1）。`/plans` 为非 Immediate 指令——Session 忙碌时在当前 LLM 调用结束后执行。

> **交叉引用**：指令语法和 plan 浏览语义详见 [mode §F6](mode.md)（plan 浏览与管理）。

### F13. 审批指令

Owner 通过 `/approve-once`、`/approve-whitelist`、`/deny` 将 Agent 操作单次放行、加入白名单或拒绝。三条指令均为 Immediate，仅 Owner 可用。

> **交叉引用**：审批决策的完整语义（单次放行/加入白名单/拒绝）、审批 ID 规则、超时参数详见 [permission §F5](permission.md)（审批工作流）。
> **交叉引用**：非 Owner 调用审批指令的权限不足提示详见 [gateway §F5](gateway.md)（斜杠指令拦截与分派）。

### F14. 调试日志

Slash 模块在以下环节记录调试日志：
- 指令匹配与分派结果
- 指令执行起止与耗时
- 指令执行异常

> **交叉引用**：日志框架定义（格式、级别、追踪标识、存储轮转、隐私脱敏）详见 [debug_log](debug_log.md)（调试日志）。

## 关联设计文档

- [✓] slash/README.md
- [✓] slash/mode-switching.md
- [✓] slash/plan-browse.md
- [✓] slash/session-management.md
- [✓] slash/status.md
- [✓] slash/compact.md
- [✓] slash/system-append.md
- [✓] slash/workdir.md
- [✓] slash/exec.md
- [✓] slash/help.md
- [✓] slash/reasoning.md
- [✓] slash/verbose.md

## 非功能需求

- Immediate 指令在 LLM 运行中必须可达，调用方不感知延迟：/stop、/status、/mode（无参数查询形态）、/reasoning、/verbose、/help

> **交叉引用**：审批指令（/approve-once、/approve-whitelist、/deny）的 Immediate 可达性详见 [gateway §F5](gateway.md)（斜杠指令拦截与分派）。
