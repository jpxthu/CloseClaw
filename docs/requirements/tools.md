# Tools 需求

## 概述

Agent 需要通过工具与外界交互——读取文件、修改代码、执行命令、调用外部服务。工具系统为 Agent 提供统一的能力发现、描述注入和安全执行通道，确保 Agent 知道有哪些工具可用、理解如何正确使用、并且操作在安全边界内执行。

## 功能需求

### F1. 工具注册与发现

Agent 需要在对话开始时就了解自己可用的工具，并在必要时按需获取更多细节。

#### 工具范围

Agent 可使用的工具范围由配置档案中的工具白名单与工具黑名单控制。新建 Agent 的白名单默认包含文件读写类工具与 shell 工具，其余工具需显式授予。初始 Agent 的工具白名单不限制，其工具范围覆盖全部工具。

> **交叉引用**：工具范围与权限规则如何共同决定可用工具，以及 spawn 工具的例外，详见 [permission §F2](permission.md)（权限维度）。
> **交叉引用**：初始 Agent 的创建规则详见 [agent §F5](agent.md)（初始 Agent 创建）。
> **交叉引用**：工具范围配置的 Session 生效边界详见 [session §F1](session.md)（对话持久化与恢复）。
> **交叉引用**：配置变更的检测与重载详见 [config §F4](config.md)（配置重载）。

#### 工具清单与检索

- Agent 在 System Prompt 中看到一份工具清单，包含每个工具的名称、分组归属与说明内容（说明内容的详细程度按下方常用 / 延迟两级划分）
- 工具清单按**常用 / 延迟**两级呈现，划分依据是工具所属的类别，与工具由哪个模块注册无关；一个工具可跨多个类别，只要其中任一类别属常用即按常用呈现：
  - **常用工具**：类别固定为以下几类，其完整说明直接呈现给 Agent，无需额外操作即可调用：
    - 文件读取（[F2](#f2-文件读取)（文件读取）；图片读取工具除外）
    - 文件写入、编辑与版本控制（[F3](#f3-文件写入与编辑)（文件写入与编辑））
    - Shell 命令执行（[F4](#f4-shell-命令执行)（Shell 命令执行））
    - 后台任务执行（同一命令工具的 `run_in_background` 参数，以及后台任务列表、终止与结果读取能力，见 [F5](#f5-后台任务执行)（后台任务执行））
    - 子 Session 管理（创建、发送任务与终止见 [session §F4](session.md)（子 Session 委托与协调））
    - 工具检索入口（见本节「Agent 可以通过关键词、工具名或要完成的事情查找工具」）
  - **延迟工具**：类别不属上述常用类别的全部工具，含本节的权限状态查询；[F9](#f9-工具扩展接入)（工具扩展接入）中各模块注册的工具按同一类别规则判定，不因注册来源改变归属。仅展示名称和风险标记，Agent 可按需查询完整说明
- Agent 可以通过关键词、工具名或要完成的事情查找工具——系统按相关度匹配，最多返回 5 个最相关的工具；按准确工具名检索时直接返回该工具。检索结果同样只包含当前可见范围内的工具
- 工具清单总长度上限 20,000 字符（含分组标注与风险标记），分组按固定顺序输出（常用分组在前、延迟分组在后）。超出时按分组从尾部截断——一个分组要么完整输出、要么整组不输出——并提示 Agent 通过工具检索了解未展示的工具（含被截断的常用工具）；若按固定顺序排在最后的分组自身就超过总长上限，则该分组内部按工具顺序从尾部截断，不使清单为空
- 单条工具说明不超过 2,000 字符（超出部分在注册时截断并标注为已截断，这是工具说明自身的长度上限）；工具检索返回该工具声明的完整说明
- 每个工具带三项风险标记，随清单一并展示（常用工具与延迟工具均展示），供 Agent 判断工具性质；标记定义见本节「风险标记」
- Agent 看到的工具清单根据当前权限动态过滤，无权使用的工具不出现在清单中
- Agent 强行调用不可见工具时，按工具不可用的拒绝提示返回（见 [F12](#f12-权限拒绝提示)（权限拒绝提示））
- spawn 工具的可用性是上述可见范围规则的例外（见 [permission §F2](permission.md)（权限维度））
- Agent 可以查询当前的权限状态，了解自己能执行哪些操作（权限状态的范围见 [permission §F2](permission.md)（权限维度））

#### 风险标记

风险标记向 Agent 提示工具的性质，共三项，各自独立判定、可同时成立；只有「只读」与「破坏性」互斥：

- **只读**：只读取数据、不产生任何副作用
- **破坏性**：产生不可逆的状态修改（写入、删除、覆盖、对外发送、提交推送等）
- **对外访问**：跨越本机边界（访问网络、调用外部服务、对外发送消息）

三项均不成立时（该工具至多产生可逆副作用、且不跨本机边界）不标注；例如提交并推送同时带 `(destructive)` 与 `(external)`。

清单中分别呈现为 `(read-only)`、`(destructive)`、`(external)`。标记由工具在注册时声明（见 [F9](#f9-工具扩展接入)（工具扩展接入））；未声明、字段缺失或取值非法时按最保守方式呈现——同时标为破坏性与对外访问。

风险标记与 [F6](#f6-并发工具调用)（并发工具调用）的并发安全分类是两个独立维度，各自声明、互不推导。标记只用于向 Agent 提示工具性质，不改变权限判定的结果、不随权限状态变化（权限判定见 [permission §F2](permission.md)（权限维度））。

面向 Agent 的提示词原文（产品定义，开发不改写），三类分列：**工具说明**（随工具定义呈现）、**使用引导**（System Prompt 工具区注入）、**错误与提示**（运行时返回或注入消息）。

**工具说明**

- `ToolSearch` — `Search available tools by keyword, exact name, or a description of what you want to do; returns the best matches with their full descriptions.`
- `PermissionQuery` — `Report the operation categories the current agent may or may not use.`

**使用引导**

- `Deferred tools show only their name and risk marker. Call ToolSearch before using one — describe what you want to do, or pass the exact tool name. When the tool list is truncated, find the unlisted tools the same way.`

**错误与提示**（`<K>` 为实际数量）

- 检索零命中 — `No matching tools. Describe the task differently, or search by the exact tool name.`
- 工具清单截断（附在清单末尾）— `Tool list truncated: <K> more tools not shown. Use ToolSearch by name or keyword to find them.`

> **交叉引用**：模块工具的接入方式。详见 [F9](#f9-工具扩展接入)（工具扩展接入）。

> **交叉引用**：工具清单随 System Prompt 组装刷新，两次组装之间不响应注册中心变更。详见 [system_prompt §F6](system_prompt.md)（内容缓存与自动刷新）。

> **交叉引用**：工具范围配置在 Agent 档案中的承载维度详见 [agent §F1](agent.md)（Agent 配置档案）。
> **交叉引用**：不可见工具调用的权限分流详见 [permission §F5](permission.md)（审批工作流）。

### F2. 文件读取

Agent 需要读取文件内容来理解代码、配置和数据。

- Agent 可以读取指定路径的文本文件，读取结果附带行号，行号与文件实际行一一对应——行号是 Agent 定位内容和指定编辑位置的基准
- Agent 可指定读取范围：`offset`（起始行，缺省从第 1 行开始）与 `limit`（读取行数，缺省读到上限或文件末尾），分段消费文件内容
- 单次读取受两处停止上限约束，任一项触及即停止：2,000 行、合计 50 KB（按 UTF-8 字节计）；另设单行字符上限 2,000 字符，它只作用于单行、不触发读取停止
- 触发行数或合计字节上限时，Agent 收到明确的续读指引——告知已读行范围、剩余行数和下一步从哪行继续（续读以 `offset` 推进），可通过续读实现完整内容读取
- 单行超长时只截断该行并标注该行的原始字符长度，其余行照常返回，不因单行超长而使整次读取失败；被截断行的剩余字符不经续读补全，需要该行完整内容时 Agent 改用搜索或命令工具按字符范围取回
- Agent 在同一轮对话中以相同参数重复读取相同文件且文件未变更时，系统告知文件未变更而非重新返回内容（去重仅在同一轮对话内生效，新一轮对话重新返回内容）
- 图片读取是独立工具（PNG/JPEG/WebP/GIF），与文本读取是两个工具：读图不常用，独立成工具、接入后按延迟工具处理（见 [F1](#f1-工具注册与发现)（工具注册与发现）），不受文本读取的行号与分段读取规则约束；当前该工具不在工具清单中，Agent 不可见也不可调用（见本节末条）
- 图片格式按文件内容特征识别，不依赖文件扩展名——无扩展名的路径也能正确识别
- 读取前自动压缩过大的图片：长边超过 2000 像素时缩放到 2000 像素（硬上限）；压缩后编码体积仍超过模型单请求大小限制的，读取被拒绝并告知原因；当前 Session 所选模型不支持图片输入时，图片读取被拒绝并告知原因
- 遇到既非文本也非支持格式图片的二进制文件时，文本读取被拒绝并告知检测到的类型；若为支持格式的图片且图片工具已接入，提示改用图片读取工具
- Agent 可以列出目录下的条目（非递归），浏览文件组织（文件名、大小、类型等基本信息）；单次返回条目数上限 200 条，超出时截断并告知总数
- Agent 可以在文件中按正则表达式搜索内容，快速定位关键内容；单次命中数上限 250 条，超出时截断并告知总命中数
- 文件路径支持相对路径（基于 Session 所属 Agent-User 组合的 workspace 解析，见 [permission §F3](permission.md)（权限决策模型））和用户主目录缩写（`~`）的展开

- 图片读取工具当前不注册进工具清单、不对 Agent 可见：读取、格式检测、压缩与本节错误文案均已定义，但 Agent 无法在清单中看到或调用该工具

面向 Agent 的提示词原文（产品定义，开发不改写），三类分列：**工具说明**（随工具定义呈现）、**使用引导**（System Prompt 工具区注入）、**错误与提示**（运行时返回或注入消息）。

**工具说明**

- `Read` — `Read a text file and return its content with line numbers.`
- `ReadImage` — `Read a PNG/JPEG/WebP/GIF image and return the image itself. The format is detected from the file content, not the file extension. Large images are downscaled to fit the model's input limit. Requires the current model to accept image input.`（图片工具可见时启用）
- `Ls` — `List the entries in a directory.`
- `Grep` — `Search file contents with a regular expression; returns matching lines with line numbers.`

**使用引导**

- `Use Read — not shell commands like cat — to inspect text files. Results are line-numbered; use offset and limit to continue through a large file.`
- `Use Ls and Grep — not shell ls or grep — for directory listing and content search.`
- `Use ReadImage to look at an image; it downscales large images, so do not install image libraries or build thumbnails.`（图片工具可见时适用）

**错误与提示**（`<path>` 为实际路径，`<type>` 为检测到的类型，`<regex>` 为正则表达式，`<start>`/`<end>`/`<total>`/`<N>` 为实际数值）

- 文件不存在 — `cannot read "<path>": not found`
- 路径不是普通文件 — `cannot read "<path>": not a regular file`
- 二进制文件 — `cannot read "<path>": binary file (<type>)`；若为支持格式的图片、且图片工具已接入，追加 `; use ReadImage to view it`
- 文件为空（提示，非错误）— `The file is empty.`
- 单次读取截断 — `(Showing lines <start>-<end> of <total>. Use offset=<end+1> to continue.)`
- 单行截断（附在该行末尾，`<N>` 为该行原始字符数）— `... (line truncated; original length <N> chars)`
- 起始行越界 — `offset <start> is out of range for "<path>" (<total> lines)`
- 同轮重复读取命中去重（提示，非错误）— `File unchanged since the last read; reuse the earlier result.`
- 目录不存在 — `cannot list "<path>": not found`
- 路径不是目录 — `cannot list "<path>": not a directory`
- 目录条目截断 — `(Showing <N> of <total> entries.)`
- 搜索正则非法 — `invalid regular expression: <regex>`
- 搜索命中截断 — `(Found <N> of <total> matches.)`
- 模型不支持图片输入（图片工具可见时适用）— `cannot read "<path>" as an image: the current model does not accept image input`
- 图片编码后超请求大小限制（图片工具可见时适用）— `cannot read "<path>" as an image: the image exceeds the model's request size limit; provide a smaller image and retry`

### F3. 文件写入与编辑

Agent 需要创建新文件、覆盖已有文件，以及对已有文件做精确的文本修改。

- Agent 可以创建新文件或覆盖已有文件的全部内容，文件不存在时自动创建父目录
- Agent 可以对已有文件做精确的文本替换——指定要被替换的原文片段和替换后的新文本
- 一次编辑可以包含多个替换操作，所有替换基于文件原始内容做匹配、互不干扰；相互重叠的替换条目被拒绝
- 被替换的原文默认需在文件中精确匹配到一处
- 精确匹配失败时尝试模糊匹配，减少因格式差异导致的匹配失败。模糊匹配仅做字符层面的归一化（空白字符、引号形态、Unicode 规范形式），不做语义近似——避免匹配到错误位置
- 匹配零处时操作被拒绝
- 匹配多处时操作被拒绝，除非 Agent 明确要求替换全部匹配处（`replace_all`）
- Agent 编辑或覆盖已存在的文件前，须在本 Session 内读取过该文件（任意轮次读取均满足）——未读取过的文件的编辑被拒绝，提示先读取再编辑
- Agent 读取文件后、执行修改前，文件被本对话之外的修改（如外部工具、用户手动编辑）改动过时，本次修改被拒绝（检测机制由设计文档定义），提示文件已变更、须重新读取后再修改
- 同一文件的多次编辑操作串行执行；不同文件的操作互不影响
- 分次编辑同一文件时，后一次编辑会因文件已被前一次编辑改动而被拒绝，Agent 需重新读取文件后再编辑
- Agent 写入文件的行尾风格（换行符）按提交内容原样写入，系统不做自动转换
- Agent 可以进行版本控制操作——查看变更状态和提交历史、提交修改、推送和拉取代码（写操作的权限判定见 [permission §F2](permission.md)（权限维度））

面向 Agent 的提示词原文（产品定义，开发不改写），三类分列：**工具说明**（随工具定义呈现）、**使用引导**（System Prompt 工具区注入）、**错误与提示**（运行时返回或注入消息）。

**工具说明**

- `Write` — `Create a file or fully replace its contents. Creates parent directories as needed.`
- `Edit` — `Edit an existing text file by replacing literal text.`
- `Git`（GitStatus / GitLog / GitCommit / GitPush / GitPull）— `Inspect working-tree status and history, and commit, push, and pull.`

**使用引导**

- `Use Write to create a file or replace its whole contents; read an existing file first — an overwrite of a file you have not read is rejected. Prefer Edit for a partial change; it sends only the changed text.`
- `Edit replaces literal old text with new text; by default each old text must appear exactly once. If one appears more than once, widen it or set replace_all. All replacements match the original content, not after earlier edits. Read the file first — a file changed since it was read must be re-read.`
- `Merge several distinct changes to one file into a single Edit call; every replacement matches the original content and none overlap. Keep each old text as small as it can be while staying unique.`
- `Use the Git tools for status, history, commit, and push/pull — do not hand-assemble git commands. Write commit messages that describe the actual change.`

**错误与提示**（`<path>` 为实际路径，`<N>`/`<M>` 为实际数量或序号）

- 未读取先修改 — `cannot modify "<path>": the file has not been read — read it first, then retry`
- 读取后文件被改动（含分次编辑同一文件的后续一次）— `cannot modify "<path>": the file changed after it was read — read it again, then retry`
- 替换原文匹配零处 — `cannot edit "<path>": the old text was not found — the current content is attached; fix the text and retry`
- 替换原文匹配多处 — `cannot edit "<path>": the old text appears <N> times — the current content is attached; widen it to make it unique, or set replace_all`
- 编辑条目重叠 — `cannot edit "<path>": entries <N> and <M> overlap — merge them or make them disjoint`
- 全文替换成功（返回，非错误）— `Replaced <N> occurrences in "<path>".`

> **交叉引用**：Agent 对文件的读/写权限判定。详见 [permission §F2](permission.md)（权限维度）。

### F4. Shell 命令执行

Agent 需要执行 Shell 命令来完成构建、测试、搜索等需要进程交互的操作。

- Agent 可以执行 Shell 命令，命令以 Session 所属 Agent-User 组合的 workspace 为工作路径（见 [permission §F3](permission.md)（权限决策模型））；实际执行位置（沙盒内或宿主上）由沙盒模式决定，Agent 可在单次调用中指定（指定与沙盒模式分流的优先关系见 [permission §F12](permission.md)（沙盒））；workspace 不可用时命令被拒绝
- 命令的标准输出与标准错误合并为单一输出流返回，输出内附带退出码，Agent 据此完整了解命令的执行结果
- Agent 可以为命令设置超时时间，上限 600 秒；超过上限的设置被截断到上限并在返回中告知（提示文案以毫秒呈现）
- Agent 设置了超时时间时，命令按该时长执行，达到时转入后台继续运行；未设置时由系统阻塞预算（120 秒）兜底，达到后同样转入后台。转入后台期间 Agent 不被阻塞等待，结果中告知命令仍在执行（含任务标识），完成后收到通知（详见 [F5](#f5-后台任务执行)（后台任务执行））
- 显式以后台方式提交的命令不受超时时间与系统阻塞预算影响
- 命令不设总执行时长上限：Session 存续期间的超长命令不被系统按时长强制终止；Session 停止时前台阻塞执行与后台运行中的命令一并终止，Session 销毁时其输出一并回收（见 [F5](#f5-后台任务执行)（后台任务执行））
- 命令输出超出显示上限（30,000 字符）时，保留输出前段，完整输出持久化到文件，Agent 收到文件路径、原始大小和输出开头预览，可按需读取完整内容（读取本身受 [F2](#f2-文件读取)（文件读取）的上限约束，单行超长时按 F2 用命令工具取回）
- 命令执行结束后（无论退出码），已因上述上限而生成的持久化输出文件，其存放位置与生命周期与后台任务输出文件一致（详见 [F5](#f5-后台任务执行)（后台任务执行）「输出文件生命周期」）；未超上限的命令不生成输出文件；超时转后台的命令按 [F5](#f5-后台任务执行)（后台任务执行）的后台任务规则处理
- 对于非零退出码不代表失败的命令（如 grep 未命中、diff 发现差异），Agent 看到的执行结果中附带退出码的语义说明，避免误判为命令失败。语义说明由系统内置的「命令 → 退出码含义」对照表提供，无内置语义的非零退出码按失败呈现
- Agent 不使用 sleep 命令做延迟等待或定时轮询（该约束面向 Agent 的文案见 [F5](#f5-后台任务执行)（后台任务执行）的使用引导）

面向 Agent 的提示词原文（产品定义，开发不改写），三类分列：**工具说明**（随工具定义呈现）、**使用引导**（System Prompt 工具区注入）、**错误与提示**（运行时返回或注入消息）。

**工具说明**

- `Bash` — `Execute a shell command in the agent workspace and return its combined stdout/stderr together with the exit code. You may choose sandbox or host execution for each call; the agent's sandbox mode and command permissions govern whether the request is allowed. Each call runs in a fresh shell: no state (variables) persists. Long output is truncated to its head; the full output is saved to a file whose path is returned.`
- （后台执行与超时转后台的文案见 [F5](#f5-后台任务执行)（后台任务执行）；命令进入审批等待期间的 Agent 侧处置见 [F8](#f8-命令结构解析)（命令结构解析））

**使用引导**

- `Run independent commands as separate calls in the same message; chain dependent commands in a single call (&&, or ; when the earlier result does not matter). Keep temporary files in the system temp directory; do not hardcode a path.`
- `Check the exit code on every result; on failure read its meaning and the error output, fix the cause, then retry — do not retry blindly.`

**错误与提示**（`<cmd>` 为命令首词，`<dir>` 为 workspace 路径，`<path>` 为输出文件路径，`<N>` 为实际数值）

- 命令不存在 — `command not found: <cmd>`
- workspace 不可用 — `cannot run in "<dir>": directory not found`
- 超时参数超上限 — `timeout capped at <N>ms`
- 输出截断 — `[output truncated; full output: <path> (<N> bytes)]`
- 非零退出码，无内置语义（默认按失败呈现）— `[exit code: <N>]`
- 非零退出码但有内置语义（非失败呈现，如 grep 未命中）— `[exit code: <N>: <meaning>]`

### F5. 后台任务执行

长时间运行的命令不应阻塞 Agent 的后续工作。

- 系统提示中告知 Agent 哪些场景应使用后台执行（预计运行时间长的命令）
- Agent 可以显式将命令提交为后台任务，无需等待即可继续执行后续操作
- Agent 在下一轮对话开始时看到当前 Session 中正在运行的后台任务列表
- Agent 可以终止指定的后台任务；被 Agent 主动终止的任务不再发送终态通知——终止结果在工具返回中已告知
- Agent 不应主动轮询检查后台任务是否完成——完成后会自动收到通知
- 后台任务到达终态（完成/失败）后，Agent 在下一轮对话开始时自动收到终态通知（含任务标识、终态、退出码、输出位置）；被 Agent 主动终止与 Session 停止引发的终止不发通知（见下条与 F10 说明）
- Session 停止引发的批量终止不发通知——Session 停止后无通知接收方
- 后台任务运行中被交互式提示（如确认对话框）卡住时，Agent 收到警告通知，建议终止该任务并以非交互方式重新运行（如用管道输入自动应答）；同一任务不重复告警
- Session 停止时，该 Session 的所有后台任务进程被终止，已产生的输出保留可读

面向 Agent 的提示词原文（产品定义，开发不改写），三类分列：**工具说明**（随工具定义呈现）、**使用引导**（System Prompt 工具区注入）、**通知与回执**（后台任务的返回与注入消息）。

**工具说明**

- 后台执行 — `Start a long command with run_in_background: the call returns a task id and an output file path immediately; the output file can be read; a running task can be stopped by its id.`

**使用引导**

- `Run a command expected to be long in the background instead of blocking — the call returns immediately and you are notified when it finishes. Do not poll or sleep on a background task, and do not duplicate a running task's work.`
- `Before a final answer, read the output of every still-relevant background task; stop tasks that no longer matter. Do not use sleep for delays or timed retries.`

**通知与回执**（逐字模板，`<task_id>` 为任务标识，`<cmd>` 为命令摘要，`<path>` 为输出文件路径，`<N>` 为退出码，`<tail>` 为阻塞时的尾部输出，`<K>` 为数量）

- 后台提交返回（返回，非错误）— `started background task <task_id>; output: <path>`
- 超时自动转后台（返回，非错误）— `moved to the background as <task_id>; output: <path>`
- 终态通知（完成）— `background task <task_id> finished [exit code: <N>]. Output: <path>`
- 终态通知（失败）— `background task <task_id> failed [exit code: <N>]. Output: <path>`
- 终态通知（被终止；Agent 主动终止与 Session 停止不发通知）— `background task <task_id> was terminated. Output: <path>`
- 卡住告警 — `background task <task_id> looks blocked on an interactive prompt (tail: <tail>) — stop it and rerun non-interactively (pipe the answers, add -y).`
- 终止返回 — `requested cancellation of task <task_id>; output so far: <path>`
- 终止不存在的任务 — `no such background task: <task_id>`
- 运行中列表注入（每轮开始；无运行中任务时不注入）— `Background tasks (<K> running):\n- <task_id>: <cmd>`

#### 输出文件生命周期

输出文件按 Session 隔离存放于系统临时目录（存放位置随 Session 销毁一并回收，不留存在用户目录），各阶段行为如下：

- 任务运行中：输出持续写入
- 任务进入终态（完成/失败/被终止）后：输出保留可读，直至 Session 销毁，不受 Session 停止影响
- Session 销毁时：该 Session 全部后台任务的输出（含各终态）一并回收，无需 Agent 手动清理
- Session 存续期间网关重启后：输出保留，仍可回读
- 系统重启后：输出不可恢复

> **交叉引用**：后台任务通知与 Session 消息队列的对接。详见 [session §F9](session.md)（消息注入）。

> **交叉引用**：/stop 是 Session 停止的指令入口。详见 [slash §F3](slash.md)（Session 管理）。

> **交叉引用**：Session 销毁的触发定义。详见 [session §F6](session.md)（Session 归档与清理）。

### F6. 并发工具调用

Agent 需要能够同时发起多个独立的工具调用，减少等待时间。

- Agent 可以在同一个消息中发起多个无依赖关系的工具调用，系统并行执行
- 多个文件读取操作可以并行执行
- 针对不同文件的文件修改操作可以并行执行
- 读取某文件的同时编辑另一个文件可以并行执行
- 读取和编辑同一文件时，读取先于编辑执行
- 每个工具声明自身的并发安全分类，分为三类：
  - **并发安全**：只读无副作用，可任意并行
  - **需串行**：对共享资源有副作用，同一工具的多次调用之间排队
  - **资源消耗型**：并发安全但消耗大，需限流

  未声明、声明字段缺失或取值非法的工具一律按需串行处理
- 资源消耗型工具的推荐并发上限由工具声明附带（如不超过 3），Agent 按声明控制并发数量

面向 Agent 的使用引导提示词原文（产品定义，开发不改写；注入位置：System Prompt 工具区）：

- `Issue independent tool calls together in one message — several reads, edits to different files, a read of one file and an edit of another. A call that depends on an earlier result waits for it. Do not send parallel edits to the same file. Limit a resource-heavy tool to its declared concurrency.`

### F7. 工具使用引导

Agent 需要理解每个工具在什么场景下使用、如何高效使用。

- 面向 Agent 的工具说明与使用引导为产品定义的固定文案，不随权限状态、运行中任务等易变信息变化
- 错误与提示、通知与回执的文案本身固定（逐字文案见各功能域），内容随具体事件填充；承载易变状态的注入消息（如运行中的后台任务列表）按对应功能域的时机发出
- 使用引导能提示 Agent 如何组合使用——哪些工具搭配效率更高
- 使用引导能指出常见误用模式——哪些操作看似正确但有问题
- 工具失败时的反馈附带可行动的修复指引——告知失败原因和下一步怎么做（逐字文案见各功能域的错误与提示段）
- 匹配失败时 Agent 无需额外发起读取即可修正重试；读取后文件已被改动的失败除外（须重新读取后再修改）
- 编辑类工具匹配失败（匹配零处或多处）时附带当前文件内容：文件不超过 50 行时附全文；超过 50 行时，匹配零处附文件开头 50 行、匹配多处附匹配位置前后各 25 行

> **交叉引用**：可调用的工具范围按当前权限过滤。详见 [F1](#f1-工具注册与发现)（工具注册与发现）。

> **交叉引用**：操作超出权限范围（含可读写的目录范围）被拦截时的分流（直接拒绝 / 进入审批）、等待与回调通知机制。详见 [permission §F5](permission.md)（审批工作流）。

> **交叉引用**：承载运行中后台任务列表的注入消息。详见 [F5](#f5-后台任务执行)（后台任务执行）。

### F8. 命令结构解析

Shell 命令在执行前需要经过结构解析。

- 解析无法确定的命令（如结构不完整、含无法判定的展开）标记为可疑，其处置按沙盒模式分流（见 [permission §F2](permission.md)（权限维度）、[permission §F12](permission.md)（沙盒））
- 解析无法确定的命令单独记录日志到 Agent 不可访问的隔离位置，日志包含触发消息与完整对话记录的标识引用（据此可回查原始上下文），供安全调查
- 解析确定的命令按命令行维度的处理顺序执行（见 [permission §F2](permission.md)（权限维度）、[permission §F12](permission.md)（沙盒））
- 命中高风险操作名单的路径操作按高风险处置（见 [permission §F7](permission.md)（高风险操作管控））

> **交叉引用**：命令级权限审批的入队、投递、等待和回调机制。详见 [permission §F5](permission.md)（审批工作流）。
> **交叉引用**：命令权限拒绝的提示句式。详见 [F12](#f12-权限拒绝提示)（权限拒绝提示）。

### F9. 工具扩展接入

系统的不同模块需要能够向工具注册中心添加自己的工具。

- Session 模块、技能系统、IM adapter 等模块可以将自己的工具注册到统一注册中心，Agent 在 System Prompt 中看到它们（技能系统注册的是技能分发工具；技能本身不携带工具权限，见 [skills §F3](skills.md)（技能配置））
- 工具注册遵循统一规范，新模块加入无需改动既有工具系统
- 每个工具都带并发安全分类（见 [F6](#f6-并发工具调用)（并发工具调用））与三项风险标记（见 [F1](#f1-工具注册与发现)（工具注册与发现）），由其定义方声明；新模块加入时按其声明自动获得正确的调度与清单呈现，无需改动既有工具系统
- 声明缺失或取值非法不阻止注册：并发安全分类按需串行、风险标记按破坏性加对外访问兜底（口径见 [F1](#f1-工具注册与发现)（工具注册与发现）、[F6](#f6-并发工具调用)（并发工具调用））
- 注册阶段的工具名冲突被检测并报错，冲突的工具注册失败

### F10. 调试日志

工具系统在以下环节记录调试日志：
- 每次工具调用（工具名、参数摘要、返回结果、耗时）
- 权限检查结果（评估结果的记录口径见 [permission §F11](permission.md)（调试日志））
- 后台任务启动与到达终态（完成/失败/被终止）
- 命令结构解析结果（可疑命令的隔离日志按 [F8](#f8-命令结构解析)（命令结构解析） 独立处理）

> **交叉引用**：子 Session 创建与完成的日志事件。详见 [session §F12](session.md)（调试日志）。

> **交叉引用**：日志框架定义详见 [debug_log §F1](debug_log.md)（完整消息链路追踪）、[debug_log §F2](debug_log.md)（分层日志级别）、[debug_log §F3](debug_log.md)（日志存储与保留）、[debug_log §F4](debug_log.md)（隐私保护）。

### F11. 面向 Agent 的行为约束提示词（跨工具域）

各功能域已内嵌本域工具的提示词原文（工具说明 / 使用引导 / 错误与提示或通知与回执）；并发调用的使用引导归入 F6，后台执行的使用引导归入 F5——同一约束只在一处定义，其他位置引用。本节收录其余**跨工具域**行为约束（产品定义，非实现细节），每条标注注入位置。

**子 Session 委派**（注入位置：spawn 工具及子 Session 管理工具的系统提示词引导，子 Session 交互详见 [session §F4](session.md)（子 Session 委托与协调））：

- `Delegated work auto-announces when it finishes. Keep doing independent work while it runs; do not poll its status or re-query the subagent list.`
- `Do not invent or predict its result before it arrives — if asked, say the work is still running.`
- `If a result arrives after your final answer, do not summarize it again — stay silent.`
- `A task you delegate must be self-contained: the child cannot see this conversation. State the goal, the context, the directions already ruled out, and the expected output format; the same applies when delegating by role.`

### F12. 权限拒绝提示

工具操作被拒绝时，Agent 需要知道自己被哪个权限维度挡住、具体是哪个对象被拒绝，以便换一条路径尝试或告知 Owner，而不是反复重试同一操作。

- 所有权限类拒绝使用同一句式，只替换操作类型、被拒对象与拒绝原因，不按场景另写一套；非权限类失败（路径不存在、类型不符、格式不支持等）按各功能域自己的文案返回，不套用本句式
- 提示点名具体对象——命令给出实际命令、文件操作给出实际路径、工具给出工具名，并给出拒绝原因
- 权限直接拒绝和审批未获放行时，均使用上述统一句式返回
- 工具不可用时同样使用该句式（见 [F1](#f1-工具注册与发现)（工具注册与发现））
- 命令解析无法确定的命令，其拒绝提示同样使用上述统一句式；按沙盒模式的分流（进入审批 / 直接拒绝 / 直接执行）见 [permission §F2](permission.md)（权限维度）、[permission §F12](permission.md)（沙盒）

面向 Agent 的提示词原文（产品定义，开发不改写）：

**拒绝提示**（`<operation>` 为被拒的操作，`<target>` 为具体命令、路径或工具名，`<permission>` 为缺少的权限维度）

- 统一句式 — `permission denied: <operation> "<target>" — <reason>`
- `<operation>` 取值：
  - `read`（读取路径）
  - `write`（写入路径）
  - `execute`（执行命令）
  - `use`（调用工具）
- `<permission>` 取值与 [permission §F2](permission.md)（权限维度）的维度对应：
  - `read`（读）
  - `write`（写）
  - `command`（命令行）
  - `network`（网络访问）
  - `config`（配置写入）
  - `messaging`（收发消息）

  多个维度同时缺失时并列列出 — `the <permission> and <permission> permissions are missing`

  工具调用维度不取 `<permission>` 值：工具不可用时改按 `<reason>` 的原因呈现
- `<reason>` 取值及对应文案：
  - 工具不可用 — `the tool is not available to this agent`
  - 权限不足 — `the <permission> permission is missing`（沙盒模式、高风险约束、审批未获放行导致的拒绝同样取本项，只换对应的权限维度）

> **交叉引用**：审批的触发条件、等待与回调机制。详见 [permission §F5](permission.md)（审批工作流）。

> **交叉引用**：拒绝提示不得暴露的内部信息边界。详见 [permission §F8](permission.md)（安全边界）。

## 非功能需求

- 文件读取、工具发现等高频工具操作在用户感知上瞬时完成
- 工具清单的总长度不能无限增长——超出时优雅截断，并给出通过工具检索了解未展示工具的指引
- Shell 命令的超时参数须有上限值，防止 Agent 设定过长超时；命令不设单条总执行时长上限，其终止与输出回收由 Session 生命周期兜底（Session 停止时一并终止、Session 销毁时回收输出）
- 后台任务不留孤儿资源——Session 停止时正在运行的后台任务一并终止，Session 销毁时输出文件无残留
- 命令结构解析不应引入明显的执行前延迟
