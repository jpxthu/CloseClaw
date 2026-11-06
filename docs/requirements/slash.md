# Slash 需求

## 概述

Owner 和 User 可以通过以 `/` 开头的消息发送斜杠指令，这些指令不进入 LLM 对话流程，由 Gateway 拦截并分派执行。

Slash 模块仅提供指令入口与分派机制；跨模块指令的业务功能由对应功能模块定义，本文档以交叉引用指向权威定义，不重复定义。

## 功能需求

### F1. 斜杠指令入口

Owner 和 User 可以通过 IM 平台、Owner 可以通过 CLI 发送以 `/` 开头的消息；消息作为斜杠指令被处理。指令的处理规则：
- 无法识别的指令：向调用者返回错误提示，引导使用 `/help` 查看可用指令
- 参数缺失或非法：除各功能域另有规定外，返回用法提示且不执行本次指令
- 业务实体不存在：按对应功能模块的定义处理

各功能域标注为「仅 Owner 可用」的指令不向非 Owner User 开放，非 Owner 调用时收到权限不足提示。

各指令的 Immediate 标记在后续各功能域中标注；未标注 Immediate 的指令默认为非 Immediate。本文档中的「调用者」指发起该指令的 Owner 或 User。

> **交叉引用**：入站身份准入先于指令分派与错误提示。详见 [gateway §F2](gateway.md)（入站消息预处理）。
> **交叉引用**：斜杠指令的拦截、分派与 Immediate 排队语义由 Gateway 负责。详见 [gateway §F5](gateway.md)（斜杠指令拦截与分派）。
> **交叉引用**：不以 `/` 开头的普通消息路由详见 [gateway §F4](gateway.md)（普通消息路由到对话）。

### F3. Session 管理

Owner 和 User 可以创建新 Session，以及终止当前 Session 的运行。

**指令**：
- `/new`：创建新 Session（即使当前对话已有未归档 Session，也强制新建）
- `/stop`：终止当前 Session 运行（Immediate）

> **交叉引用**：Session 创建与恢复详见 [session §F1](session.md)（对话持久化与恢复）。
> **交叉引用**：Session 停止时前台命令与后台任务的终止规则详见 [tools §F4](tools.md)（Shell 命令执行）、[tools §F5](tools.md)（后台任务执行）。
> **交叉引用**：停止运行与 Session 归档、保留的关系详见 [session §F6](session.md)（Session 归档与清理）。
> **交叉引用**：workflow 模式下的终止语义详见 [workflow §F6](workflow.md)（流程暂停与恢复）。
> **交叉引用**：workflow 模式退出（生命周期与清理）详见 [workflow §F8](workflow.md)（流程生命周期）。

### F4. 状态查询

Owner 和 User 可以随时查看当前 Session 的状态与用量。

**指令**：
- `/status`：查询 Session 状态、用量与供应商账户信息（Immediate）

#### 查询内容
- 模型
- 推理强度
- 上下文窗口占用率
- 缓存命中率
- token 用量
- 预估费用
- 供应商账户信息（余额、用量配额）
- 未完成子 Session 数
- System Prompt 追加指令列表（仅 Owner）

#### 输出规则
除 System Prompt 追加指令列表仅 Owner 可见外，其余查询项对所有调用者可见。

> **交叉引用**：Session 的模型取值详见 [llm §F11](llm.md)（Session 模型选择）。
> **交叉引用**：推理强度见 [llm §F4](llm.md)（推理强度控制）。
> **交叉引用**：上下文窗口占用率、缓存命中率、token 用量与预估费用的计算口径见 [llm §F9](llm.md)（用量统计）。
> **交叉引用**：供应商账户信息的查询范围、支持情况与失败处置见 [llm §F1](llm.md)（多供应商统一对话）。
> **交叉引用**：子 Session 的未完成判定详见 [session §F11](session.md)（Session 活跃维度）。
> **交叉引用**：子 Session 完成的判定条件详见 [session §F7](session.md)（运行健康与安全）。
> **交叉引用**：追加指令见 [system_prompt §F5](system_prompt.md)（追加指令管理）。

### F5. 上下文压缩

Owner 和 User 可以手动触发对话历史的上下文压缩。

**指令**：
- `/compact`：不带保留指示的压缩
- `/compact <保留指示>`：携带保留指示的压缩

#### 保留指示
保留指示为 `/compact` 之后的整段自然语言，去掉首尾空白。

- 保留指示内容（去除首尾空白后）为空：等同于 `/compact`
- 超过 500 字符（按字符数计）：直接拒绝并返回用法提示，不截断
- 作用范围：只影响摘要的重点，不改变压缩范围与保留原文的规则

> **交叉引用**：压缩引擎的压缩范围和行为定义在 [session §F3](session.md)（长对话压缩）。

### F6. System Prompt 追加

Owner 可以在运行时管理 System Prompt 追加区的追加指令，无需修改配置文件。多次追加的内容叠加保留，不覆盖已有追加。该功能仅 Owner 可用。

**指令**：
- `/system add <内容>`：追加一条指令
- `/system` 或 `/system list`：列出当前所有 Owner 追加指令
- `/system clear`：清除全部 Owner 追加指令

#### 限制
- 追加内容超过 500 字符（按字符数计，不计首尾空白）：直接拒绝并返回用法提示，不截断
- `/system add` 不带内容：返回用法提示

> **交叉引用**：追加区在 System Prompt 中的位置由 [system_prompt §F5](system_prompt.md)（追加指令管理）定义。
> **交叉引用**：追加内容的存储和持久化由 [session §F2](session.md)（恢复时的追加指令保留）定义。

### F8. 命令执行

`/exec` 仅 Owner 可用，用于执行 Shell 命令。命令在当前 Session 所属 Agent-User 组合的 workspace 路径下执行。

**指令**：
- `/exec <命令>`：执行 Shell 命令

> **交叉引用**：workspace 的定义详见 [permission §F3](permission.md)（权限决策模型）。
> **交叉引用**：Owner 的命令执行豁免详见 [permission §F4](permission.md)（Owner 特权）。
> **交叉引用**：命令的黑名单、白名单与执行位置详见 [permission §F2](permission.md)（权限维度）。
> **交叉引用**：斜杠指令的沙盒约束详见 [permission §F12](permission.md)（沙盒）。

### F9. 帮助

Owner 和 User 可以查看斜杠指令的帮助。

**指令**：
- `/help`：查看帮助（Immediate）

#### 角色过滤
- Owner：看到全部指令及其说明
- 非 Owner User：只看到未标注「仅 Owner 可用」的指令；标注为「仅 Owner 可用」的指令不出现在帮助结果中，帮助结果也不提示存在未展示的指令

#### 动态生成
帮助内容从系统当前支持的指令自动生成，系统升级引入的新指令自动出现在帮助中。

### F10. 推理强度控制

Owner 和 User 可以查询和设置当前 Session 的 LLM 推理强度。查询与设置结果回显实际生效档位。

**指令**：
- `/reasoning`（无参数）：查询当前推理强度档位（Immediate）
- `/reasoning low|medium|high|max|off`：设置推理强度档位（Immediate）

> **交叉引用**：推理强度的档位定义、默认值、降级策略、非法值处理及 `off` 的行为见 [llm §F4](llm.md)（推理强度控制）。

### F11. 展示等级

Owner 和 User 可以查询和设置当前 Session 的展示等级。非法等级值被忽略，回显当前生效等级。

**指令**：
- `/verbose`（无参数）：查询当前展示等级（Immediate）
- `/verbose full|normal|off`：设置展示等级（Immediate）

> **交叉引用**：各展示等级对应的回复内容、默认值、临时覆盖及切换生效范围见 [processor_chain §F4](processor_chain.md)（出站回复展示等级控制）。

### F13. 审批指令

Owner 通过审批指令审批 Agent 的文件访问与命令执行授权。三条指令均为 Immediate，仅 Owner 可用。

**指令**：
- `/approve-once <单号>`：单次放行
- `/approve-whitelist <单号>`：加入白名单
- `/deny <单号>`：拒绝

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

仅 Owner 可通过 `/workflow <名称>` 启动对应的 workflow。`/workflow` 为非 Immediate 指令：满足排队条件时排队等待，待排队条件解除后执行。启动语义由 workflow 模块定义。

> **交叉引用**：排队条件的定义详见 [session §F10](session.md)（消息排队）。
> **交叉引用**：启动语义与 workflow 模式进入详见 [workflow §F2](workflow.md)（workflow 启动）。
> **交叉引用**：workflow 模式退出（生命周期与清理）详见 [workflow §F8](workflow.md)（流程生命周期）。

### F16. 权限配置管理

以下指令仅 Owner 可用，用于新增权限规则，并设置沙盒模式。`/perm` 不带子指令时返回用法提示，列出可用子指令。

**指令**：
- `/perm allow <规则主体> <作用维度> [生效对象]`：新增一条允许规则
- `/perm deny <规则主体> <作用维度> [生效对象]`：新增一条拒绝规则
- `/perm sandbox <默认|严格|放行>`：设置当前 Session 所属 Agent 的沙盒模式（沙盒模式按 Agent 维度配置）

#### 参数规则
- 规则主体写法：`agent:<Agent ID>` 或 `user:<User ID>`
- 作用维度：取七个权限维度名称原文之一
- 生效对象：整维度生效的维度不带 `<生效对象>` 参数
- 沙盒模式取值：`默认`、`严格`、`放行`（分别对应 permission §F12 的默认模式、严格模式、放行模式）

> **交叉引用**：七个权限维度的定义详见 [permission §F2](permission.md)（权限维度）。
> **交叉引用**：规则主体与生效对象的约束、修改时的 Owner 校验与生效机制详见 [permission §F6](permission.md)（权限配置管理）。
> **交叉引用**：高风险对象的限制详见 [permission §F7](permission.md)（高风险操作管控）。
> **交叉引用**：沙盒模式的语义与差异详见 [permission §F12](permission.md)（沙盒）。

## 非功能需求

- 标注为 Immediate 的指令不受排队条件限制，调用者无需等待当前 Session 正在进行的推理与工具调用结束。

> **交叉引用**：Immediate 指令的可达性由 Gateway 的排队决策保证。详见 [gateway §F5](gateway.md)（斜杠指令拦截与分派）。
