# Slash 需求

## 概述

Owner 和 User 可以通过以 `/` 开头的消息发送斜杠指令，这些指令不进入 LLM 对话流程，由 Gateway 拦截并分派执行。

Slash 模块仅提供指令入口与分派机制；跨模块指令的业务功能由对应功能模块定义，本文档以交叉引用指向权威定义，不重复定义。

## 功能需求

### F1. 斜杠指令入口

Owner 和 User 发送以 `/` 开头的消息时，消息作为斜杠指令被处理。指令的处理规则：
- 无法识别的指令：向调用者返回错误提示，引导使用 `/help` 查看可用指令
- 参数缺失或非法：除各功能域另有规定外，返回用法提示且不执行本次指令；业务实体不存在等情形按对应业务模块的定义处理

各功能域标注为「仅 Owner 可用」的指令不向非 Owner User 开放，非 Owner 调用时收到权限不足提示。

各指令的 Immediate 标记在后续各功能域中标注；未标注 Immediate 的指令默认为非 Immediate。

> **交叉引用**：斜杠指令的拦截、分派与 Immediate 排队语义由 Gateway 负责。详见 [gateway §F5](gateway.md)（斜杠指令拦截与分派）。
> **交叉引用**：不以 `/` 开头的普通消息路由详见 [gateway §F4](gateway.md)（普通消息路由到对话）。

### F3. Session 管理

Owner 和 User 可以创建新 Session，以及终止当前 Session 运行。

**指令**：
- `/new`：创建新 Session（即使当前标识下已有未归档 Session，也强制新建）
- `/stop`：终止当前 Session 运行（Immediate）

> **交叉引用**：Session 创建与恢复详见 [session §F1](session.md)（对话持久化与恢复）。
> **交叉引用**：workflow 模式下的终止语义详见 [workflow §F6](workflow.md)（流程暂停与恢复）。
> **交叉引用**：workflow 模式退出（生命周期与清理）详见 [workflow §F8](workflow.md)（流程生命周期）。

### F4. 状态查询

Owner 和 User 可以随时查看当前 Session 的状态与用量。

**指令**：
- `/status`：查询 Session 状态与用量（Immediate）

#### 查询内容
- 模型
- 推理强度
- 上下文窗口占用率
- 缓存命中率
- token 用量
- 预估费用
- 未完成子 Session 数
- System Prompt 追加指令列表（仅 Owner）

#### 输出规则
- 除 System Prompt 追加指令列表仅 Owner 可见外，其余查询项对所有调用者可见
- 预估费用无法计算时显示为不可用，不显示为 0

> **交叉引用**：Session 的模型取值详见 [agent §F6](agent.md)（运行时配置查询）。
> **交叉引用**：推理强度见 [llm §F4](llm.md)（推理强度控制）。
> **交叉引用**：上下文窗口占用率、缓存命中率、token 用量与预估费用的计算口径见 [llm §F9](llm.md)（用量统计）。
> **交叉引用**：子 Session 的未完成判定详见 [session §F11](session.md)（Session 活跃维度）。
> **交叉引用**：追加指令见 [system_prompt §F5](system_prompt.md)（追加指令管理）。

### F5. 上下文压缩

Owner 和 User 可以手动触发对话历史的上下文压缩。

**指令**：
- `/compact`：不带保留指示的压缩
- `/compact <保留指示>`：携带保留指示的压缩

#### 保留指示
保留指示为 `/compact` 之后的整段自然语言，去掉首尾空白。

- 内容为空：等同于 `/compact`
- 超过 500 字符（按字符数计，不计首尾空白）：直接拒绝并返回用法提示，不截断
- 作用范围：只影响摘要的重点，不改变手动压缩的行为

> **交叉引用**：压缩引擎的压缩范围和行为定义在 [session §F3](session.md)（长对话压缩）。

### F6. System Prompt 追加

Owner 可以在运行时向 System Prompt 的追加区动态添加指令，无需修改配置文件。多次追加的内容叠加保留，不覆盖已有追加。该功能仅 Owner 可用。

**指令**：
- `/system add <内容>`：追加一条指令
- `/system` 或 `/system list`：列出当前所有追加指令
- `/system clear`：清除全部追加指令

#### 限制
- 追加内容超过 500 字符（按字符数计，不计首尾空白）：直接拒绝并返回用法提示，不截断
- `/system add` 不带内容：返回用法提示

> **交叉引用**：追加区在 System Prompt 中的位置由 [system_prompt §F5](system_prompt.md)（追加指令管理）定义。
> **交叉引用**：追加内容的存储和持久化由 [session §F2](session.md)（恢复时的追加指令保留）定义。

### F7. Git 操作

Owner 和 User 可以执行 Git 命令。Git 命令以当前 Session 所属 Agent-User 组合的 workspace 为工作目录执行。

**指令**：
- `/git <参数>`：执行 Git 命令
  - 只读子命令（status、log、diff、show，以及不带参数的 branch）无需权限评估与审批（黑名单命令除外）
  - 其余 Git 子命令按权限规则评估与审批；Owner 调用不进入权限评估与审批（黑名单命令除外）

> **交叉引用**：Owner 的命令执行豁免详见 [permission §F4](permission.md)（Owner 特权）。
> **交叉引用**：Git 写操作的权限评估详见 [permission §F3](permission.md)（权限决策模型）。
> **交叉引用**：Git 写操作的审批详见 [permission §F5](permission.md)（审批工作流）。
> **交叉引用**：命令维度的黑名单、白名单与执行位置详见 [permission §F2](permission.md)（权限维度）。
> **交叉引用**：斜杠指令的沙盒约束详见 [permission §F12](permission.md)（沙盒）。

### F8. 命令执行

`/exec` 仅 Owner 可用，用于执行 Shell 命令。命令以当前 Session 所属 Agent-User 组合的 workspace 为工作目录执行。

**指令**：
- `/exec <命令>`：执行 Shell 命令

> **交叉引用**：Owner 的命令执行豁免详见 [permission §F4](permission.md)（Owner 特权）。
> **交叉引用**：命令的黑名单、白名单与执行位置详见 [permission §F2](permission.md)（权限维度）。
> **交叉引用**：斜杠指令的沙盒约束详见 [permission §F12](permission.md)（沙盒）。

### F9. 帮助

Owner 和 User 可以查看斜杠指令的帮助。

**指令**：
- `/help`：查看帮助（Immediate）

#### 角色过滤
- Owner：看到全部指令及其说明
- 非 Owner User：只看到自身可用的指令；标注为「仅 Owner 可用」的指令不出现在帮助结果中，帮助结果也不提示存在未展示的指令

#### 动态生成
帮助内容从系统当前支持的指令自动生成，系统升级引入的新指令自动出现在帮助中。

### F10. 推理强度控制

Owner 和 User 可以查询和设置当前 Session 的 LLM 推理强度。查询与设置结果回显实际生效档位——模型不支持所请求档位时自动降级。非法档位值被忽略，回显当前生效档位。

**指令**：
- `/reasoning`（无参数）：查询当前推理强度档位（Immediate）
- `/reasoning low|medium|high|max|off`：设置推理强度档位（Immediate）

> **交叉引用**：推理强度的等级定义、默认值、降级策略、非法值处理及 `off` 的行为见 [llm §F4](llm.md)（推理强度控制）。

### F11. 展示等级

Owner 和 User 可以查询和设置当前 Session 的展示等级。设置不影响正在生成的回复，仅对其后开始的回复生效。非法等级值被忽略，回显当前生效等级。

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

`/approve-once`、`/approve-whitelist` 的主体参数可取 `agent`、`user` 或 `both`，未指定时按 `both` 处理。命令类操作只涉及 Agent 维度（`both` 即 `agent`），显式指定 `user` 时返回用法提示。

> **交叉引用**：审批决策的完整语义（单次放行/加入白名单/拒绝）、审批单号的全局唯一性与命中规则、投递目标详见 [permission §F5](permission.md)（审批工作流）。

### F14. 调试日志

Slash 模块在以下环节记录调试日志：
- 指令匹配结果（命中 / 未命中）
- 指令执行起止与耗时
- 指令执行异常

> **交叉引用**：日志框架定义详见 [debug_log §F1](debug_log.md)（完整消息链路追踪）。
> **交叉引用**：分层日志级别详见 [debug_log §F2](debug_log.md)（分层日志级别）。
> **交叉引用**：日志存储与保留详见 [debug_log §F3](debug_log.md)（日志存储与保留）。
> **交叉引用**：隐私保护详见 [debug_log §F4](debug_log.md)（隐私保护）。

### F15. workflow 指令

Owner 通过 `/workflow <名称>` 启动对应的 workflow，仅 Owner 可用。`/workflow` 为非 Immediate 指令：满足排队条件时排队等待，待排队条件解除后执行。启动语义由 workflow 模块定义。

> **交叉引用**：排队条件的定义详见 [session §F10](session.md)（消息排队）。
> **交叉引用**：启动语义与 workflow 模式进入详见 [workflow §F2](workflow.md)（workflow 启动）。
> **交叉引用**：workflow 模式退出（生命周期与清理）详见 [workflow §F8](workflow.md)（流程生命周期）。

### F16. 权限配置管理

以下指令仅 Owner 可用，用于查看、新增与删除权限规则，并设置沙盒模式。`/perm` 不带子指令时返回用法提示，列出可用子指令。

**指令**：
- `/perm list`：列出系统中全部当前生效的权限规则，从 1 开始按列表顺序连续编号
- `/perm allow <规则主体> <作用维度> [生效对象]`：新增一条允许规则
- `/perm deny <规则主体> <作用维度> [生效对象]`：新增一条拒绝规则
- `/perm remove <序号>`：删除 Owner 最近一次 `/perm list` 输出中对应序号的规则；其后规则发生增删时该序号失效，返回错误提示且不执行，需重新列出后再操作
- `/perm sandbox <默认|严格|放行>`：设置当前 Session 所属 Agent 的沙盒模式；修改即时生效，无需重启

#### 参数规则
- 规则主体写法：`agent:<Agent ID>` 或 `user:<User ID>`
- 作用维度：取 permission §F2 的七个维度名称原文之一
- 生效对象：整维度生效的维度不带 `<生效对象>` 参数
- 沙盒模式取值：`默认`、`严格`、`放行`

> **交叉引用**：七个权限维度的定义详见 [permission §F2](permission.md)（权限维度）。
> **交叉引用**：规则主体与生效对象的约束、高风险对象的限制、修改时的 Owner 校验与生效机制详见 [permission §F6](permission.md)（权限配置管理）。
> **交叉引用**：沙盒模式的语义与差异详见 [permission §F12](permission.md)（沙盒）。

## 非功能需求

- 标注为 Immediate 的指令在 LLM 运行中仍可被受理，调用者无需等待当前 LLM 调用结束。

> **交叉引用**：Immediate 指令的可达性由 Gateway 的排队决策保证。详见 [gateway §F5](gateway.md)（斜杠指令拦截与分派）。
