# slash 需求

## 概述

通过以 `/` 开头的消息，User 和 Owner 可以发送系统控制指令，这些指令不进入 LLM 对话流程，由 Gateway 拦截并分派给对应的指令处理器执行。部分指令标记为 Immediate —— 可在 LLM 运行时被立即响应；非 Immediate 指令需等待当前 LLM 调用结束。

## 功能需求

### F1. 斜杠指令入口

User 或 Owner 发送以 `/` 开头的消息时，消息不被路由到 LLM 对话流程，由 Gateway 拦截并解析为指令名和参数，分派给对应的指令处理器执行。不以 `/` 开头的消息正常路由到 LLM 对话。无匹配处理器的指令向 User 返回友好错误提示，引导使用 `/help` 查看可用指令。

各指令的 Immediate 标记见后续各功能域的指令列表中标注。

> **交叉引用**：所有以 `/` 开头的消息统一由斜杠指令系统拦截和分派，包括 `/approve`、`/deny` 等审批类指令。审批流程本身由 Permission 模块负责，详见 [permission §F2](permission.md)（权限维度）。指令拦截和响应优先级由 Gateway 负责，详见 [gateway §F5](gateway.md)（斜杠指令拦截与分派）。

### F2. 模式切换

slash 模块提供 Normal ↔ Plan 模式的切换入口。完整模式类型定义（含 Auto Mode）和模式间的业务行为约束由 mode 模块定义，详见 [mode §F1](mode.md)（运行模式）。`/execute` 命令触发 Auto Mode 执行，入口语法由 slash 模块定义。模式切换不立即变更 system prompt——切换仅标记会话状态，下一条用户消息进入 LLM 前生效。

**指令**：
- `/plan`：切换到 Plan 模式。不接受附加参数，附加的文本会被忽略，不进入 LLM 对话
- `/mode`（无参数）：查询当前模式（Immediate）
- `/mode plan`：等价于 `/plan`
- `/mode normal`：切换到 Normal 模式
- `/mode <非法值>`：提示错误，模式不变

> **交叉引用**：Plan 模式下的 Agent 行为约束定义在 [mode §F1](mode.md)（运行模式定义）。

### F3. 会话管理

Owner 和 User 可以创建新会话，以及强行终止当前 Agent 运行。

**指令**：
- `/new`：创建新会话
- `/stop`：终止当前 LLM 调用及所有子 Agent（Immediate）

> **交叉引用**：会话创建、停止、归档的完整生命周期定义在 [session §F1](session.md)（对话持久化与恢复）与 [session §F4](session.md)（子 Agent 委托与协调）。

### F4. 状态查询

Owner 和 User 可以随时查看当前会话的运行状态。该指令为 Immediate。

**指令**：
- `/status`：查询会话状态（展示模式、模型、推理深度、上下文用量、缓存命中率与 token 累计、活跃子 Agent 数、工作目录、system prompt 追加指令列表）

> **交叉引用**：展示内容的各字段定义分散在对应模块中——模式见 [mode §F1](mode.md)、推理深度见 [llm §F4](llm.md)、会话统计和子 Agent 见 [session](session.md)、工作目录见 [session §F8](session.md)、追加指令见 [system_prompt §F5](system_prompt.md)。

### F5. 上下文压缩

Owner 和 User 可以手动触发对话历史的上下文压缩。

**指令**：
- `/compact`：默认压缩
- `/compact <保留指示>`：携带保留指示的压缩

> **交叉引用**：压缩引擎的压缩范围和行为定义在 [session §F3](session.md)（长对话压缩）。

### F6. system prompt 追加

Owner 和 User 可以在运行时向 system prompt 的追加区动态添加指令，无需修改配置文件。多次追加叠加不覆盖。

**指令**：
- `/system add <内容>`：追加一条指令
- `/system` 或 `/system list`：列出当前所有追加指令
- `/system clear`：清除全部追加指令

追加内容超过 500 字符时，直接拒绝并向 User 返回错误提示，不进行截断。`/system add` 不带内容时，向 User 返回用法提示。

> **交叉引用**：追加区在 system prompt 中的位置由 [system_prompt §F5](system_prompt.md)（动态指令管理）定义；追加内容的存储和持久化由 [session §F2](session.md)（Agent 角色与能力配置）定义。

### F7. 工作目录操作

Owner 和 User 可以变更和查看当前会话的工作目录，以及执行 Git 命令。

**指令**：
- `/cd <路径>`：变更工作目录，切换前校验路径存在性，不存在时提示错误；成功后回复路径及 Git 分支信息
- `/pwd`：查看当前工作目录
- `/git <args>`：执行 Git 命令。只读子命令（status、log、diff、branch、show）直接执行，写操作需经权限审批

> **交叉引用**：工作目录的定义（字段、默认值、变更后的状态展示、Git 命令输出）见 [session §F8](session.md)（工作目录）。Git 写操作的权限审批见 [permission §F3](permission.md)（权限决策模型）。

### F8. 命令执行

Owner 可以执行任意 Shell 命令。User 默认无权限（可由 Owner 通过权限规则授权）。

**指令**：
- `/exec <命令>`：执行 Shell 命令

> **交叉引用**：命令执行的权限评估由 Permission 模块负责，详见 [permission §F3](permission.md)（权限决策模型）。

### F9. 帮助

Owner 和 User 可以查看所有当前可用的斜杠指令及其说明。新增指令自动出现在帮助中。

该指令为 Immediate。

**指令**：
- `/help`：查看帮助

### F10. 推理深度控制

Owner 和 User 可以查询和设置当前会话的 LLM 推理深度。

该指令为 Immediate。

**指令**：
- `/reasoning`（无参数）：查询当前推理深度
- `/reasoning low|medium|high|max|off`：设置推理深度。off 是 low 的别名

> **交叉引用**：推理深度的等级定义、默认值、优先级和降级策略见 [llm §F4](llm.md)（推理强度控制）。

### F11. 展示等级

Owner 和 User 可以查询和设置当前会话的展示等级。展示等级控制 Agent 内部工作细节向 User 展示的量。

切换等级不影响当前正在输出的消息，仅对后续新消息生效。该指令为 Immediate。

**指令**：
- `/verbose`（无参数）：查询当前展示等级
- `/verbose full|normal|off`：设置展示等级

> **交叉引用**：展示等级的过滤内容定义见 [processor_chain §F4](processor_chain.md)（出站回复冗余控制）。

## 关联设计文档

- [✓] slash/README.md
- [✓] slash/mode-switching.md
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

- Immediate 指令（/stop、/status、/mode、/reasoning、/verbose、/help 的查询形态）在 LLM 运行中必须可达，User 不感知延迟
- `/exec` 和 `/git` 写操作必须经过权限审批方可执行，不可绕过
