# slash 需求

## 概述

通过以 `/` 开头的消息，User 和 Owner 可以发送系统控制指令，这些指令不进入 LLM 对话流程，而是在 Gateway 层被拦截并直接执行。斜杠指令覆盖模式切换、会话管理、状态查询、上下文压缩、运行时配置调整、工作目录操作和命令执行等场景。

## 功能需求

### F1. 斜杠指令入口

User 发送以 `/` 开头的消息时，消息不被路由到 LLM 对话流程，由 Gateway 层拦截并解析为指令名和参数，分派给对应的指令处理器执行。不以 `/` 开头的消息正常路由到 LLM 对话。无匹配处理器的指令（如 `/unknown_cmd`）向 User 返回友好错误提示，引导使用 `/help` 查看可用指令。

执行完成的指令回复走与 LLM 回复相同的出站链路送达 User，受相同的 Verbosity 过滤、DSL 解析和平台渲染规则约束。

部分指令标记为 Immediate——即使 LLM 正在运行中也能立即响应，不被消息队列阻塞。非 Immediate 指令在 LLM 忙碌时向 User 提示等待。

> 注：`/approve`、`/deny` 不经过斜杠指令系统，由 Gateway 层直接走审批流程处理。

> **交叉引用**：指令拦截和路由决策由 Gateway 完成，详见 [gateway §F2](gateway.md)（入站消息的路由决策）。

### F2. 模式切换

Owner 和 User 可以在 Normal 和 Plan 两种会话模式之间切换。

- Plan 模式：Agent 按规划工作流执行（Research → Design → Review → Confirm），工具集限制为只读加 plan 文件写入，system prompt 中注入规划工作流指令
- Normal 模式：Agent 按标准行为工作，使用完整工具集和标准 system prompt

模式切换不立即变更 system prompt——切换仅标记会话状态，下一条用户消息进入 LLM 前生效。

**指令**：
- `/plan`：切换到 Plan 模式
- `/mode`（无参数）：查询当前模式
- `/mode plan`：等价于 `/plan`
- `/mode normal`：切换到 Normal 模式
- `/mode <非法值>`：提示错误，模式不变

`/plan` 后附加的文本（如 `/plan 设计缓存`）不传递到会话。

> **交叉引用**：Plan 模式下的 Agent 行为约束定义在 [mode §F1](mode.md)（运行模式定义）。

### F3. 会话管理

Owner 和 User 可以创建新会话，以及强行终止当前 Agent 运行。

**创建新会话**：创建新的会话标识，后续消息自动路由到新会话。旧会话保留，后续由系统自然归档。

**终止当前运行**：立即终止当前 LLM 调用及所有子 Agent，清除运行队列。即使 LLM 正在运行中也能立即响应（Immediate 指令）。

**指令**：
- `/new`：创建新会话
- `/stop`：终止当前运行

> **交叉引用**：会话创建、停止、归档的完整生命周期定义在 [session §F1](session.md)（会话生命周期）。

### F4. 状态查询

Owner 和 User 可以随时查看当前会话的运行状态。该指令为 Immediate，LLM 运行时也能响应。

**展示内容**：当前模式、模型名称、推理深度、上下文用量、缓存命中率、缓存读写 token 累计、活跃子 Agent 数、工作目录、system prompt 追加指令列表。

**指令**：
- `/status`：查询会话状态

### F5. 上下文压缩

Owner 和 User 可以手动触发对话历史的上下文压缩，释放 LLM 上下文中的 token 空间。

无参数时使用默认压缩策略。可通过参数指定保留指令，指导压缩引擎重点关注哪些内容。压缩仅作用于对话历史，静态内容保持不变。

**指令**：
- `/compact`：默认压缩
- `/compact <保留指令>`：携带保留指令的压缩

> **交叉引用**：压缩引擎的行为定义在 [session §F6](session.md)（上下文压缩）。

### F6. System Prompt 追加

Owner 和 User 可以在运行时向 system prompt 的追加区动态添加指令，无需修改配置文件。

追加区是 system prompt 末尾的独立分区，位于动态层之后、对话历史之前，与 AGENTS.md 等静态内容互不干扰。多次追加叠加，不覆盖；持久化在会话状态中，会话恢复时保留；不受上下文压缩影响。

**指令**：
- `/system add <内容>`：追加一条指令
- `/system` 或 `/system list`：列出当前所有追加指令
- `/system clear`：清除全部追加指令

追加内容超过 500 字符时，向 User 返回错误提示，不截断。`/system add` 不带内容时，向 User 返回用法提示。

> **交叉引用**：追加区的 prompt 组装行为定义在 [system_prompt §F3](system_prompt.md)（动态层与追加区）。

### F7. 工作目录操作

Owner 和 User 可以变更和查看当前会话的工作目录，以及执行 Git 命令。

**变更工作目录**：切换工作目录前校验路径存在性，不存在时提示错误，存在时切换并展示路径和 Git 分支信息。

**查看工作目录**：输出当前工作目录路径。

**Git 操作**：执行 Git 命令，只读子命令（status、log、diff、branch、show）直接执行，写操作需经权限审批。

**指令**：
- `/cd <路径>`：变更工作目录
- `/pwd`：查看当前工作目录
- `/git <args>`：执行 Git 命令

> **交叉引用**：工作目录的定义（字段、默认值、与 system prompt 注入的关系）见 [session §F8](session.md)（工作目录）。

### F8. 命令执行

Owner 可以执行任意 Shell 命令。非 Owner 默认无权限（可由 Owner 通过权限规则授权）。

命令经权限模块评估通过后方可执行；被拒绝时向 User 返回权限不足提示。

**指令**：
- `/exec <命令>`：执行 Shell 命令

> **交叉引用**：命令执行的权限评估由 Permission 模块负责，详见 [permission §F3](permission.md)（命令执行权限）。

### F9. 帮助

Owner 和 User 可以查看所有可用斜杠指令及其说明。帮助文本由系统根据已注册指令动态生成，新增指令自动出现，无需手动维护。

该指令为 Immediate，LLM 运行时也能响应。

**指令**：
- `/help`：查看帮助

### F10. 推理深度控制

Owner 和 User 可以查询和设置当前会话的 LLM 推理深度。推理深度控制 LLM 在生成回复前的内部推理量。

提供四个等级：Low、Medium、High、Max，默认为 High。`off` 是 Low 的别名。运行时设置优先级高于全局配置默认值。不支持的等级由 Provider 侧自动降级。

该指令为 Immediate，LLM 运行时也能响应。

**指令**：
- `/reasoning`（无参数）：查询当前推理深度
- `/reasoning low|medium|high|max|off`：设置推理深度

> **交叉引用**：推理深度的模型映射由 LLM 模块负责，详见 [llm §F5](llm.md)（推理深度映射）。

### F11. 信息展示等级

Owner 和 User 可以查询和设置当前会话的信息展示等级。展示等级控制 Agent 内部工作细节向 User 展示的量，不影响 LLM 推理深度，不影响 Agent 行为模式。

三个等级：
- full（默认）：展示全部——思考过程、工具调用、工具结果、最终回复
- normal：展示工具调用和结果作为进度提示，隐藏思考过程
- off：仅展示最终回复，隐藏所有中间过程

切换等级不影响当前正在输出的消息，仅对后续新消息生效。

该指令为 Immediate，LLM 运行时也能响应。

**指令**：
- `/verbose`（无参数）：查询当前展示等级
- `/verbose full|normal|off`：设置展示等级

> **交叉引用**：展示等级的过滤行为由出站 Processor Chain 负责，详见 [processor_chain §F4](processor_chain.md)（Verbosity 过滤）。

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

- 斜杠指令的响应延迟不受 LLM 排队影响，Immediate 指令（/stop、/status、/reasoning、/verbose、/help）即使在 LLM 运行中也必须及时响应
- `/exec` 和 `/git` 写操作必须经过权限审批方可执行，不可绕过
- 所有斜杠指令的回复必须走与 LLM 回复相同的出站链路，确保 Verbosity 过滤和平台渲染规则一致生效
