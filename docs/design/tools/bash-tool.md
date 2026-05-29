# Bash 工具

## 概述

BashTool 为 agent 提供执行 shell 命令的能力，是 agent 与本地开发环境交互的核心工具。支持命令执行、超时控制、输出截断与持久化、后台执行，并可通过权限引擎进行命令级安全管控。

## 架构

BashTool 的执行链路包含五层处理：

```
输入参数解析
  → 安全解析（AST 分析 + 信任分级，详见 bash-security.md）
    → 权限校验
      → 子进程执行（前台/后台分支）
        → 输出累积与截断
          → 结果组装（返回给 agent 或持久化引用）
```

### 参数

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `command` | string | 是 | 要执行的 shell 命令，原样传递给 shell |
| `timeout` | number | 否 | 超时毫秒数，默认 120,000（2 分钟），最大 600,000（10 分钟） |
| `description` | string | 否 | 命令用途的简短描述，用于进度展示和通知摘要。始终用主动语态，不包含"复杂""危险"等主观词 |
| `run_in_background` | boolean | 否 | 设为 true 将命令放入后台执行，立即返回任务标识。agent 随后通过通知获取结果 |
| `cwd` | string | 否 | 工作目录，默认为 session 的工作目录 |
| `dangerouslyDisableSandbox` | boolean | 否 | 仅当沙箱已启用时有效。设为 true 将绕过此次调用的沙箱限制 |

### 超时控制

命令执行有硬性时间限制。超时后子进程被终止，结果中标记 `interrupted: true`，已产生的 stdout/stderr 保留。

默认超时适应用户配置（环境变量可覆盖默认值和上限），但上限不可超过 10 分钟。超过 10 分钟的长任务应使用 `run_in_background` 后台化。

agent 调用时也可手动指定更短或更长的超时（不超上限），以适应特定命令的预期执行时间。

此外，agent 运行时层可触发自动后台化：当命令在前台运行超过一定时间（阻塞预算），agent 运行时将其转为后台任务继续执行，BashTool 返回 `assistantAutoBackgrounded: true` 和任务标识。

### 输出累积与截断

命令执行期间，stdout 和 stderr 合并为单一数据流，通过渐进式累积器处理。

累积器采用**保留头部、丢弃尾部**的策略：当累积字符数超过阈值（默认 30,000 字符），新到达的输出被丢弃，尾部标注已移除的字节数。这确保 agent 始终看到命令输出的**开头部分**——通常是编译错误、测试结果、构建状态等最关键信息。

### 输出持久化

命令输出完整写入磁盘文件。当累积器中保存的文本不足以反映全貌时（磁盘文件大小超过内存累积阈值），输出不以原始文本形式返回给 agent，而是：

1. 完整输出保存到工具结果目录
2. agent 收到一个 `<persisted-output>` 引用，包含文件路径、原始大小、前 2,000 字节预览
3. agent 需要完整输出时，通过文件读取工具按需加载

持久化文件硬上限 64MB，超出部分截断。

### 命令分类

BashTool 根据命令的第一个词对命令进行语义分类，用于 UI 展示优化：

| 类别 | 示例命令 | 展示行为 |
|------|---------|---------|
| 搜索（search） | grep、find、rg、ag、ack、locate、which、whereis | 可折叠，摘要显示命中数 |
| 读取（read） | cat、head、tail、less、more、wc、jq、awk | 可折叠，摘要显示读取行数 |
| 列表（list） | ls、tree、du | 可折叠，摘要显示条目数 |
| 静默（silent） | mv、cp、rm、mkdir、rmdir、chmod、chown、chgrp、touch、ln、cd、export、unset、wait | 成功时显示"Done"而非"(No output)" |
| 语义中立（neutral） | echo、printf、true、false、: | 不影响管道的读/写判定 |

对于管道命令，只有当所有环节都属于搜索/读取/列表/中立类别时，整条命令才可折叠。

### 进度展示

命令执行超过 2 秒后，UI 显示实时进度信息：已输出行数、字节数、已耗时。进度信息随着输出到达持续更新，直到命令完成。

### 权限

BashTool 自身标记为破坏性工具和昂贵工具，在工具索引中显示对应的危险度标记。

命令级别的权限校验在工具执行前由权限引擎完成。BashTool 将命令和参数提交给权限引擎的**命令行**维度检查，权限引擎根据白名单配置决定放行或拒绝。

### 沙箱

当系统沙箱启用时，BashTool 在受限环境中执行命令：文件系统读写范围受限、网络访问受控。agent 可通过 `dangerouslyDisableSandbox` 参数单次绕过沙箱限制，但该参数需要对应的权限审批。

### 输出结构

BashTool 的 call() 返回结果包含以下字段：

| 字段 | 类型 | 说明 |
|------|------|------|
| `stdout` | string | 标准输出（≤ 累积阈值时为完整内容，> 阈值时为 `<persisted-output>` 引用） |
| `stderr` | string | 标准错误输出 |
| `exitCode` | number | 退出码，0 表示成功 |
| `interrupted` | boolean | 是否因超时被终止 |
| `backgroundTaskId` | string | 后台任务标识（仅 `run_in_background: true` 或自动后台化时返回） |
| `assistantAutoBackgrounded` | boolean | 是否因阻塞预算超时被自动转为后台 |
| `backgroundedByUser` | boolean | 是否被用户手动转为后台 |
| `persistedOutputPath` | string | 持久化输出文件路径（输出超过累积阈值时） |
| `persistedOutputSize` | number | 持久化文件的原始字节数 |
| `returnCodeInterpretation` | string | 非错误退出码的特殊语义说明（如 grep 的 0=命中、1=未命中） |
| `noOutputExpected` | boolean | 命令是否预期不产生输出（用于 UI 展示"Done"） |

## 数据流

### 前台执行（run_in_background = false 或未指定）

```
agent 调用 BashTool（command + 可选参数）
  → 安全解析：AST 分析 + 信任分级
    → malicious → 拦截 + 通知 owner
    → uncertain → 发起审批
    → trusted：
      → 权限引擎检查命令是否在白名单
        → 拒绝：返回权限错误
        → 通过：
          → 启动子进程，合并 stdout/stderr 到输出累积器
        → 输出流式到达，渐进累积
          → 累积器达阈值 → 丢弃新增内容，标注截断量
        → 命令结束或超时
          → 超时：发送终止信号，标记 interrupted
          → 正常完成：收集退出码
      → 输出小于累积阈值：直接返回 stdout/stderr/exitCode
      → 输出大于累积阈值：持久化到磁盘，返回 <persisted-output> 引用 + 预览
      → 返回执行结果给 agent
```

### 后台执行（run_in_background = true）

```
agent 调用 BashTool（command + run_in_background: true）
  → 同上权限校验
    → 创建后台任务，命令异步执行
    → 立即返回 backgroundTaskId + 输出文件路径给 agent
    → agent 继续处理后续任务
    → 后台命令完成后，通过通知系统向 agent 注入完成消息（机制详见 background-tasks.md）
```

### 自动后台化

```
agent 调用 BashTool（前台模式）
  → 命令开始执行
    → 阻塞预算计时器启动（15 秒）
      → 15 秒内完成：正常返回结果
      → 15 秒未完成：
        → 命令自动转为后台任务
        → 返回 assistantAutoBackgrounded: true + backgroundTaskId
        → 告知 agent 命令仍在运行，完成后会通知
```

## 模块关系

- **上游**：agent 运行时（调度工具调用、传递参数和上下文）
- **下游**：安全解析模块（AST 分析、信任分级）、权限引擎（命令白名单校验）、后台任务系统（执行后台命令、管理任务生命周期、发送完成通知）、沙箱系统（可选，限制执行环境）
- **无关**：processor_chain（不参与消息出站处理）、IM 适配器（不参与平台渲染）
