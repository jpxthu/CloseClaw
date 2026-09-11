# 工具系统 需求

## 概述

Agent 需要通过工具与外界交互——读取文件、修改代码、执行命令、调用外部服务。工具系统为 Agent 提供统一的能力发现、描述注入和安全执行通道，确保 Agent 知道有哪些工具可用、理解如何正确使用、并且操作在安全边界内执行。

## 功能需求

### F1. 工具注册与发现

Agent 需要在对话开始时就了解自己可用的工具，并在必要时按需获取更多细节。

- Agent 在 System Prompt 中看到一份工具清单，包含每个工具的名称、分组归属和功能描述
- 工具清单按**常用 / 延迟**两级呈现：
  - **常用工具**：由 [F2](#f2-文件读取)（文件读取）、[F3](#f3-文件写入与编辑)（文件写入与编辑）、[F4](#f4-shell-命令执行)（Shell 命令执行）、[F5](#f5-后台任务执行)（后台任务执行）定义的工具，子 Session 管理工具（创建见 [agent §F7](agent.md)（子 Session 创建（Spawn））、发送任务与终止见 [session §F4](session.md)（子 Session 委托与协调）），以及工具检索入口（见本节「Agent 可以通过关键词或工具名查找工具」）。其完整说明直接呈现给 Agent，无需额外操作即可调用
  - **延迟工具**：除常用工具外的全部工具，含本节的权限状态查询与 [F9](#f9-工具扩展接入)（工具扩展接入）中各模块注册的工具。至少展示名称和风险标记，Agent 可按需查询完整说明
- Agent 可以通过关键词或工具名查找工具——用自然语言描述想做的事情，系统匹配最相关的工具
- 工具清单总长度有上限（按 token 预算，默认值由设计文档定义），超出时从尾部截断并提示 Agent 如何了解未展示的工具
- Agent 看到的工具清单根据当前权限动态过滤，无权使用的工具不出现在清单中
- Agent 可以查询当前的权限状态，了解自己能执行哪些操作

面向 Agent 的提示词原文（产品定义，开发不改写），三类分列：**工具说明**（随工具定义呈现）、**使用引导**（System Prompt 工具区注入）、**错误与提示**（运行时返回或注入消息）。

**工具说明**

- `ToolSearch` — `Search available tools by keyword or exact name; returns the best matches with their full descriptions.`
- `PermissionQuery` — `Report the operation categories the current agent may or may not use.`

**使用引导**

- `Deferred tools show only their name and risk marker. Call ToolSearch before using one — describe what you want to do, or pass the exact tool name. When the tool list is truncated, find the unlisted tools the same way.`

**错误与提示**（`K` 为实际数量）

- 检索零命中 — `No matching tools. Describe the task differently, or search by the exact tool name.`
- 工具清单截断（附在清单末尾）— `Tool list truncated: K more tools not shown. Use ToolSearch by name or keyword to find them.`

> **交叉引用**：模块工具的接入方式。详见 [F9](#f9-工具扩展接入)（工具扩展接入）。

> **交叉引用**：工具清单随 System Prompt 组装刷新，两次组装之间不响应注册中心变更。详见 [system_prompt §F6](system_prompt.md)（内容缓存与自动刷新）。

> **交叉引用**：当前 Agent 可用的工具范围（白名单/黑名单）。详见 [agent §F3](agent.md)（Agent 能力组合）。

### F2. 文件读取

Agent 需要读取文件内容来理解代码、配置和数据。

- Agent 可以读取指定路径的文本文件，读取结果附带行号，行号与文件实际行一一对应——行号是 Agent 定位内容和指定编辑位置的基准
- Agent 可指定读取范围（起止行号），分段消费文件内容。单次读取有默认上限（行数与字节数双阈值，先到者生效；具体数值由设计文档定义），超出时 Agent 收到明确的续读指引——告知已读行范围、剩余行数和下一步从哪行继续，可通过续读实现完整内容读取
- Agent 在同一轮对话中以相同参数重复读取相同文件且文件未变更时，系统告知文件未变更而非重新返回内容（去重仅在同一轮对话内生效，新一轮对话重新返回内容）
- Agent 可以读取图片文件（PNG/JPEG/WebP/GIF）——图片读取与文本读取是**两个独立工具**：读图不常用，独立成工具并按延迟加载处理（见 [F1](#f1-工具注册与发现)），不占用文本读取的复杂度
- 图片格式按文件内容的文件头特征字节识别（如 PNG 的起始字节为 89 50 4E 47、JPEG 为 FF D8 FF），不依赖文件扩展名——无扩展名的路径也能正确识别
- 读取前自动压缩过大的图片：长边缩放到不超过 2000 像素（硬上限）；压缩后长边仍超 2000 像素或编码后仍超过模型单请求大小限制的，读取被拒绝并告知原因；当前 Session 所选模型不支持图片输入时，图片读取被拒绝并告知原因
- 遇到既非文本也非支持格式图片的二进制文件时，文本读取拒绝并告知检测到的类型；若是支持格式的图片，提示改用图片读取工具
- Agent 可以列出目录结构，浏览文件组织（文件名、大小、类型等基本信息）；单次返回条目数有上限（默认值由设计文档定义），超出时截断并告知总数
- Agent 可以在文件中搜索文本模式，快速定位关键内容；单次命中数有上限（默认值由设计文档定义），超出时截断并告知总命中数
- 文件路径支持相对路径（基于当前 Session 工作目录解析）和用户主目录缩写（`~`）的展开

图片读取功能的边界（阶段性约束，解除条件：LLM 层多模态设计——多模态消息格式、各 provider 协议映射、多模态对话测试 fixture——完成并落地）：

- 在上述设计补齐前，图片读取工具**只实现、不接入产品**：实现读取、格式检测、压缩的全部能力（含本节的错误文案），但不注册进工具清单、不对 Agent 可见；接入时机另行决策

面向 Agent 的提示词原文（产品定义，开发不改写），三类分列：**工具说明**（随工具定义呈现）、**使用引导**（System Prompt 工具区注入）、**错误与提示**（运行时返回）。

**工具说明**

- `Read` — `Read a text file and return its content with line numbers.`
- `ReadImage` — `Read a PNG/JPEG/WebP/GIF image and return the image itself. The format is detected from the file content, not the file extension. Large images are downscaled to fit the model's input limit. Requires the current model to accept image input.`（本工具暂不接入产品，文案随接入启用）
- `Ls` — `List the entries in a directory.`
- `Grep` — `Search file contents with a regular expression; returns matching lines with line numbers.`

**使用引导**

- `Use Read — not shell commands like cat — to inspect text files. Results are line-numbered; use offset and limit to continue through a large file.`
- `Use Ls and Grep — not shell ls or grep — for directory listing and content search.`
- `Use ReadImage to look at an image; it downscales large images, so do not install image libraries or build thumbnails.`（随图片工具接入启用）

**错误与提示**（`<path>` 为实际路径，`<type>` 为检测到的类型，`<start>`/`<end>`/`<total>`/`<N>` 为实际数值）

- 文件不存在 — `cannot read "<path>": not found`
- 路径不是普通文件 — `cannot read "<path>": not a regular file`
- 二进制文件 — `cannot read "<path>": binary file (<type>)`；若为支持格式的图片，追加 `; use ReadImage to view it`
- 文件为空（提示，非错误）— `The file is empty.`
- 单次读取截断 — `(Showing lines <start>-<end> of <total>. Use offset=<end+1> to continue.)`
- 起始行越界 — `offset <start> is out of range for "<path>" (<total> lines)`
- 同轮重复读取命中去重（提示，非错误）— `File unchanged since the last read; reuse the earlier result.`
- 目录不存在 — `cannot list "<path>": not found`
- 路径不是目录 — `cannot list "<path>": not a directory`
- 目录条目截断 — `(Showing <N> of <total> entries.)`
- 搜索正则非法 — `invalid regular expression: <regex>`
- 搜索命中截断 — `(Found <N> of <total> matches.)`
- 模型不支持图片输入（随图片工具接入启用）— `cannot read "<path>" as an image: the current model does not accept image input`
- 图片超过尺寸上限（随图片工具接入启用）— `cannot read "<path>" as an image: it exceeds the <N>px limit; downscale it and retry`
- 图片编码后超请求大小限制（随图片工具接入启用）— `cannot read "<path>" as an image: the image exceeds the model's request size limit; downscale it and retry`

### F3. 文件写入与编辑

Agent 需要创建新文件、覆盖已有文件，以及对已有文件做精确的文本修改。

- Agent 可以创建新文件或覆盖已有文件的全部内容，文件不存在时自动创建父目录
- Agent 可以对已有文件做精确的文本替换——指定要被替换的原文片段和替换后的新文本
- 一次编辑可以包含多个替换操作，所有替换基于文件原始内容做匹配，互不干扰
- 被替换的原文默认需在文件中精确匹配到一处
- 精确匹配失败时尝试模糊匹配，降低因格式差异导致的匹配失败。模糊匹配仅做字符层面的归一化（空白字符、引号形态、Unicode 规范形式），不做语义近似——避免匹配到错误位置
- 匹配零处时操作被拒绝
- 匹配多处时操作被拒绝，除非 Agent 明确要求全文替换
- Agent 编辑或覆盖已存在的文件前，须已在对话中读取过该文件——从未读取过的文件的编辑被拒绝，提示先读取再编辑
- Agent 读取文件后、执行修改前，文件被本对话之外的修改（如外部工具、用户手动编辑）改动过时，本次修改被拒绝（检测机制由设计文档定义），提示文件已变更、须重新读取后再修改
- 同一文件的多次编辑操作串行执行；不同文件的操作互不影响
- 分次编辑同一文件时，后一次编辑会因文件已被前一次编辑改动而被拒绝，Agent 需重新读取文件后再编辑
- Agent 写入文件的行尾风格（换行符）按提交内容原样写入，系统不做自动转换
- Agent 可以进行版本控制操作——查看变更状态和提交历史、提交修改、推送和拉取代码

面向 Agent 的提示词原文（产品定义，开发不改写），三类分列：**工具说明**（随工具定义呈现）、**使用引导**（System Prompt 工具区注入）、**错误与提示**（运行时返回）。

**工具说明**

- `Write` — `Create a file or fully replace its contents. Creates parent directories as needed.`
- `Edit` — `Edit an existing text file by replacing literal text.`
- `Git`（GitStatus / GitLog / GitCommit / GitPush / GitPull）— `Inspect working-tree status and history, and commit, push, and pull.`
- `Write` 参数：`file_path` — path to write; `content` — full text content
- `Edit` 参数：`file_path` — path to edit; `old_string` — literal text to replace, must match exactly; `new_string` — replacement text, empty to delete; `replace_all` — replace all matches, default false

**使用引导**

- `Use Write to create a file or replace its whole contents; read an existing file first — an overwrite of a file you have not read is rejected. Prefer Edit for a partial change; it sends only the changed text.`
- `Edit replaces literal old text with new text; by default the old text must appear exactly once. If it appears more than once, widen the old text or set replace_all. Read the file first — a file changed since it was read must be re-read.`
- `Merge several distinct changes to one file into a single Edit call; every replacement matches the original content and none overlap. Keep each old text as small as it can be while staying unique.`
- `Use the Git tools for status, history, commit, and push/pull — do not hand-assemble git commands. Write commit messages that describe the actual change.`

**错误与提示**（`<path>` 为实际路径，`<N>`/`<M>` 为实际数量或序号）

- 未读取先修改 — `cannot modify "<path>": the file has not been read — read it first, then retry`
- 读取后文件被外部改动（含分次编辑同一文件的后续一次）— `cannot modify "<path>": the file changed after it was read — read it again, then retry`
- 替换原文匹配零处 — `cannot edit "<path>": the old text was not found — the current content is attached; fix the text and retry`
- 替换原文匹配多处 — `cannot edit "<path>": the old text appears <N> times — widen it to make it unique, or set replace_all`
- 编辑条目重叠 — `cannot edit "<path>": entries <N> and <M> overlap — merge them or make them disjoint`
- 全文替换成功（返回，非错误）— `Replaced <N> occurrences in "<path>".`

> **交叉引用**：Agent 对文件的读/写权限判定。详见 [permission §F2](permission.md)（权限维度）。

### F4. Shell 命令执行

Agent 需要执行 Shell 命令来完成构建、测试、搜索等需要进程交互的操作。

- Agent 可以执行 Shell 命令，在指定的工作目录下运行；工作目录不存在时命令被拒绝
- 命令的标准输出与标准错误合并为单一输出流返回，输出内附带退出码，Agent 据此完整了解命令的执行结果
- Agent 可以为命令设置超时时间（有系统上限，超上限的设置被截断到上限并在返回中告知）；Agent 设置的超时时间或系统阈值达到时（以先到者为准），命令自动转入后台继续运行，Agent 不会被阻塞等待，转后台结果中告知 Agent 命令仍在执行（含任务标识），完成后收到通知（详见 [F5](#f5-后台任务执行)）。三层时间阈值（Agent 可设超时上限 / 系统阻塞预算 / 单命令总执行时长兜底）的具体数值由设计文档定义
- 命令输出超出显示上限（字符数阈值，默认值由设计文档定义）时，保留输出前段，完整输出持久化到文件，Agent 收到文件路径、原始大小和输出开头预览，可按需读取完整内容
- 命令执行结束后（无论退出码），该命令的持久化输出文件生命周期与后台任务输出文件一致（详见 [F5](#f5-后台任务执行) 输出文件生命周期）；超时转后台的命令按 [F5](#f5-后台任务执行) 后台任务规则处理
- 对于非零退出码不代表失败的命令（如 grep 未命中、diff 发现差异），Agent 看到的执行结果中附带退出码的语义说明，避免误判为命令失败。语义说明按命令内置（命令名 → 退出码含义），无内置语义的非零退出码按失败呈现
- Agent 不使用 sleep 命令做延迟等待或定时轮询——延迟后重试、定时检查状态属于反模式，需要等待时应使用后台执行

面向 Agent 的提示词原文（产品定义，开发不改写），三类分列：**工具说明**（随工具定义呈现）、**使用引导**（System Prompt 工具区注入）、**错误与提示**（运行时返回）。

**工具说明**

- `Bash` — `Execute a shell command in a working directory and return its combined stdout/stderr together with the exit code. Each call runs in a fresh shell: no state (working directory, variables) persists — pass the working directory as a parameter instead of using cd. Long output is truncated to its head; the full output is saved to a file whose path is returned when available.`
- 参数 `Bash`：`command` — the shell command; `timeout` — timeout in milliseconds (has a default and a cap); `workdir` — working directory, defaults to the session working directory; `description` — short purpose, shown in progress and notifications
- （后台执行与超时转后台的文案见 [F5](#f5-后台任务执行)；命令安全拦截与审批的 Agent 侧文案见 [F8](#f8-命令安全防护)）

**使用引导**

- `Run independent commands as separate calls in the same message; chain dependent commands in a single call (&&, or ; when the earlier result does not matter). Pass the working directory as a parameter — do not rely on cd. Keep temporary files in the system temp directory; do not hardcode a path.`
- `For a command expected to run long, run it in the background instead of blocking: the call returns immediately and you are notified when it finishes.`
- `Check the exit code on every result; on failure read its meaning and the error output, fix the cause, then retry — do not retry blindly.`

**错误与提示**（`<cmd>` 为命令首词，`<dir>` 为工作目录，`<path>` 为输出文件路径，`<N>` 为实际数值）

- 命令不存在 — `command not found: <cmd>`
- 工作目录不存在 — `cannot run in "<dir>": directory not found`
- 超时参数超上限 — `timeout capped at <N>ms`
- 输出截断 — `[output truncated; full output: <path>]`
- 非零退出码，无内置语义（默认按失败呈现）— `[exit code: <N>]`
- 非零退出码但有内置语义（非失败呈现，如 grep 未命中）— `[exit code: <N>: <meaning>]`

### F5. 后台任务执行

长时间运行的命令不应阻塞 Agent 的后续工作。

- 系统提示中告知 Agent 哪些场景应使用后台执行（预计运行时间长的命令）
- Agent 可以显式将命令提交为后台任务，无需等待即可继续执行后续操作
- Agent 在下一轮对话开始时看到当前 Session 中正在运行的后台任务列表
- Agent 可以终止指定的后台任务；被 Agent 主动终止的任务不再发送终态通知——终止结果在工具返回中已告知
- Agent 不应主动轮询检查后台任务是否完成——完成后会自动收到通知
- 后台任务到达终态（完成/失败/被终止）后，Agent 在下一轮对话开始时自动收到终态通知（含任务标识、终态、退出码、输出位置）
- Session 停止引发的批量终止不发通知——Session 停止后无通知接收方
- 后台任务运行中被交互式提示（如确认对话框）卡住时，Agent 收到警告通知，建议终止该任务并以非交互方式重新运行（如用管道输入自动应答）；同一任务不重复告警
- Session 停止时，该 Session 的所有后台任务进程被终止，已产生的输出保留可读

面向 Agent 的提示词原文（产品定义，开发不改写），三类分列：**工具说明**（随工具定义呈现）、**使用引导**（System Prompt 工具区注入）、**通知与回执**（后台任务的返回与注入消息）。

**工具说明**

- 后台任务工具（提交 / 读取输出 / 列出 / 终止）— `Submit a command as a background job and return its id immediately; read a job's output; list this session's jobs; stop a running job.`
- 提交参数：`command`、`workdir` — see [F4](#f4-shell-命令执行)
- 读取输出参数：`job_id` — job id; `wait` — block until a terminal status, default false
- 终止参数：`job_id` — job id; `reason` — optional reason

**使用引导**

- `Do not poll or sleep on a background job — you are notified in-session when it finishes. Keep working on independent steps, and do not duplicate a running job's work.`
- `Before a final answer, read the output of every still-relevant job; stop jobs that no longer matter. Do not use sleep for delays or timed retries.`

**通知与回执**（逐字模板，`<job_id>` 为任务标识，`<cmd>` 为命令摘要，`<path>` 为输出文件路径，`<N>` 为退出码，`<tail>` 为阻塞时的尾部输出，`<K>` 为数量）

- 后台提交返回（返回，非错误）— `started background job <job_id>`
- 超时自动转后台（返回，非错误）— `moved to the background as <job_id>; output: <path>`
- 终态通知（完成）— `background job <job_id> finished [exit code: <N>]. Output: <path>`
- 终态通知（失败）— `background job <job_id> failed [exit code: <N>]. Output: <path>`
- 终态通知（被终止）— `background job <job_id> was terminated. Output: <path>`
- 卡住告警 — `background job <job_id> looks blocked on an interactive prompt (tail: <tail>) — stop it and rerun non-interactively (pipe the answers, add -y).`
- 终止返回 — `requested cancellation of job <job_id>; output so far: <path>`
- 终止不存在的任务 — `no such background job: <job_id>`
- 运行中列表注入（每轮开始；无运行中任务时不注入）— `Background jobs (<K> running):\n- <job_id>: <cmd>`

> **交叉引用**：后台任务通知与 Session 消息队列的对接。详见 [session §F9](session.md)（消息注入）。

后台任务的输出文件按 Session 隔离存放于系统临时目录，生命周期各阶段行为如下：

- 任务运行中：输出持续写入
- 任务进入终态（完成/失败/被终止）后：输出保留可读，直至 Session 销毁，不受 Session 停止影响
- Session 销毁时：该 Session 全部后台任务的输出（含各终态）一并回收，无需 Agent 手动清理
- Session 存续期间网关重启后：输出保留，仍可回读
- 系统重启后：输出不可恢复

> **交叉引用**：/stop 是 Session 停止的指令入口。详见 [slash §F3](slash.md)（Session 管理）。
> **交叉引用**：Session 销毁的触发定义。详见 [session §F6](session.md)（Session 归档与清理）。

### F6. 并发工具调用

Agent 需要能够同时发起多个独立的工具调用，减少等待时间。

- Agent 可以在同一个消息中发起多个无依赖关系的工具调用，系统并行执行
- 多个文件读取操作可以并行执行
- 针对不同文件的文件修改操作可以并行执行
- 读取某文件的同时编辑另一个文件可以并行执行
- 读取和编辑同一文件时，读取先于编辑执行
- 每个工具声明自身的并发安全分类，完整枚举：**并发安全**（只读无副作用，可任意并行）、**需串行**（对共享资源有副作用，同类调用间排队）、**资源消耗型**（并发安全但消耗大，需限流）。未声明、声明字段缺失或取值非法的工具一律按需串行处理（fail-closed）
- 资源消耗型工具的推荐并发上限由工具声明附带（如不超过 3），Agent 按声明控制并发数量

面向 Agent 的使用引导提示词原文（产品定义，开发不改写；注入位置：System Prompt 工具区）：

- `Issue independent tool calls together in one message — several reads, edits to different files, a read of one file and an edit of another. A call that depends on an earlier result waits for it. Do not send parallel edits to the same file. Limit a resource-heavy tool to its declared concurrency.`

### F7. 工具使用引导

Agent 需要理解每个工具在什么场景下使用、如何高效使用。

- 工具说明能反映当前上下文——当权限受限时说明受限的操作范围，当工作目录确定时给出路径指引
- 工具说明能提示 Agent 如何组合使用——哪些工具搭配效率更高
- 工具说明能指出常见误用模式——哪些操作看似正确但有问题
- 工具失败时的反馈附带可行动的修复指引——告知失败原因和下一步怎么做（逐字文案见各功能域的错误与提示段），编辑类工具匹配失败时附带当前文件内容（不超过 50 行时附全文；超过时附匹配尝试位置前后各 25 行），Agent 无需额外发起读取即可修正重试
- 缺少上下文感知能力的工具说明降级为静态描述，功能不受影响
- 进入 System Prompt 工具区的说明文本保持稳定——权限状态、运行中任务等易变信息不嵌入工具说明，通过消息注入传达，避免前缀缓存反复失效

### F8. 命令安全防护

Shell 命令在执行前需要经过安全检查，防止恶意操作。

- 命令执行前检测是否有攻击模式（如注入绕过、隐藏命令），一旦发现直接拦截并通知 Owner
- 解析不确定的命令（如结构不完整）标记为可疑，发起审批请求，由 Owner 决定是否执行
- 疑似攻击或解析不确定的命令单独记录日志到 workspace 之外的专用目录，日志包含触发消息与完整对话记录的引用，供安全调查
- 结构清晰且无风险的命令正常进入白名单匹配流程
- 危险文件路径（系统关键文件、版本控制内部文件等）的操作不进入白名单，必须经 Owner 审批
- 审批请求推送给 Owner 时，附带 Agent 提供的命令用途说明，帮助 Owner 理解命令意图

命令用途说明（使用引导，面向 Agent 的提示词原文，产品定义，开发不改写）：

- `Give every command a one-sentence purpose — active voice, stating the operation (read / write / delete / modify), the target (path, command, or service), and the expected effect; five to ten words for simple commands ("list files in the current directory"). When pipes or uncommon flags would obscure it, the sentence must let the Owner judge what it does ("recursively find and delete all .tmp files"). A purpose missing any of the three elements is incomplete and may be denied; no subjective words like "complex" or "dangerous".`

**错误与提示**（`<category>` 为攻击类别，`<reason>` 为原因）

- 命令被安全拦截 — `command blocked by the security check (<category>); the Owner has been notified`
- 命令待审批 — `command entered approval (reason: <reason>); waiting for the Owner. Continue other work; do not resubmit this command.`
- 审批被拒绝 — `approval denied; the command did not run`
- 审批超时 — `approval timed out; the command did not run`

> **交叉引用**：命令级权限审批的入队、推送和回调机制。详见 [permission §F5](permission.md)（审批工作流）。

### F9. 工具扩展接入

系统的不同模块需要能够向工具注册中心添加自己的工具。

- Session 模块、技能系统、IM adapter 等模块可以将自己的工具注册到统一注册中心，Agent 在 System Prompt 中看到它们
- 工具注册遵循统一规范，新模块加入无需改动既有工具系统
- 注册阶段的工具名冲突被检测并报错

### F10. 调试日志

工具系统在以下环节记录调试日志：
- 每次工具调用（工具名、参数摘要、返回结果、耗时）
- 权限检查结果
- 后台任务启动与到达终态（完成/失败/被终止）
- 命令安全扫描结果（安全审计类事件除外，按 F8 独立处理）

> **交叉引用**：子 Session 创建与完成的日志事件。详见 [session §F12](session.md)（调试日志）。
> **交叉引用**：日志框架定义详见 [debug_log §F1](debug_log.md)（完整消息链路追踪）、[debug_log §F2](debug_log.md)（分层日志级别）、[debug_log §F3](debug_log.md)（日志存储与保留）、[debug_log §F4](debug_log.md)（隐私保护）。

### F11. 面向 Agent 的行为约束提示词（跨工具域）

F2-F6 各节已内嵌本域工具的三类提示词原文（工具说明 / 使用引导 / 错误与提示）；并发调用的使用引导归入 F6，后台执行的引导归入 F4——同一约束只在一处定义，其他位置引用。本节收录其余**跨工具域**行为约束（产品定义，非实现细节），每条标注注入位置。

**子 Session 委派**（注入位置：spawn 工具及子 Session 管理工具的系统提示词引导，子 Session 交互详见 [agent §F7](agent.md)（子 Session 创建（Spawn））、[session §F4](session.md)（子 Session 委托与协调））：

- `Delegated work auto-announces when it finishes. Keep doing independent work while it runs; do not poll its status or re-query the subagent list.`
- `Do not invent or predict its result before it arrives — if asked, say the work is still running.`
- `If a result arrives after your final answer, do not summarize it again — stay silent.`
- `A task you delegate must be self-contained: the child cannot see this conversation. State the goal, the context, the directions already ruled out, and the expected output format; the same applies when delegating by role.`

## 非功能需求

- 文件读取、工具发现等高频工具操作在用户感知上瞬时完成
- 工具清单的总长度不能无限增长——超出时优雅截断并给出探索指引，不影响 Agent 了解可用工具范围
- Shell 命令的超时参数须有上限值，防止 Agent 设定过长超时；系统对单个命令的总执行时间保留兜底终止能力
- 后台任务不留孤儿资源——Session 销毁后，其后台任务不再运行，输出文件无残留
- 命令安全检测不应引入明显的执行前延迟
