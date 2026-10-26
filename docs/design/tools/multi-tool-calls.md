# 多工具并行调用

## 概述

多工具并行调用允许 agent 在一个消息中同时发起多个工具调用，减少 round-trip，提升响应速度。系统通过并发安全声明与 per-file 互斥队列，在利用并行优势的同时防止冲突操作。

## 架构

多工具调用的保护机制分两层：

```
agent 在一个消息中发起多个工具调用
  │
  ├─ 第一层：并发分类声明
  │     → 每个工具通过 concurrency_class 声明自身分类（并发安全 / 需串行 / 资源消耗型）
  │     → 破坏性工具默认需串行（同一文件的写操作需串行）
  │     → 只读工具默认并发安全（多个 Read 可并行）
  │     → 资源消耗型并发安全但按推荐并发上限限流
  │
  └─ 第二层：per-file 互斥队列
        → 同一文件的所有修改操作串行排队
        → 不同文件的操作完全并行
        → Read 不受队列影响（只读操作无需排队）
```

### 并发分类声明

每个工具在注册时通过 `concurrency_class`（[common ToolFlags](../common/shared-types.md#工具契约载荷族)）声明自身分类——并发安全 / 需串行 / 资源消耗型；资源消耗型另声明 `recommended_concurrency`（推荐并发上限）。未声明、字段缺失或取值非法一律按需串行处理（fail-closed）：

| 工具 | 并发分类 | 原因 |
|------|---------|------|
| Read | 并发安全 | 只读，无副作用 |
| Ls | 并发安全 | 只读，无副作用 |
| Grep | 资源消耗型 | 只读但递归搜索开销大，按推荐并发上限限流 |
| Write | 需串行 | 破坏性，需 per-file 互斥 |
| Edit | 需串行 | 破坏性，需 per-file 互斥 |
| Bash | 需串行 | 破坏性，需命令级权限检查 |
| ToolSearch | 并发安全 | 只读，查询注册表无副作用 |
| sessions_spawn | 并发安全 | 创建子 session，并发数由 agent 配置 maxChildren 控制 |
| sessions_steer | 并发安全 | 操作子 session，归属检查在工具层完成 |

需串行的工具通过上层保护机制（per-file 互斥队列、配置上限、权限引擎）保证安全；资源消耗型工具不超出 `recommended_concurrency`。

### 调度器与互斥映射

多工具调度的两个领域实现件（不入 common，common 只承载载荷类型 `PendingToolCall` / `DispatchGroup`）：

- **ToolCallDispatcher**：依据并发分类与按文件互斥所有权，把一批调用路由到并行 / 按文件互斥 / 限流并行 / 串行执行；不直接调工具，由调用方注入 `ToolExecutor`
- **FileMutexMap**：同文件并发写的按路径互斥映射，key 为规范化路径，每项持锁、无引用时惰性移除；其非阻塞获取结果为 Acquired（已获取，返回 owned guard）/ WouldBlock（锁被其他任务持有）

### per-file 互斥队列

文件修改操作的并发保护核心。同一文件路径的所有 Write 和 Edit 调用串行排队——先到的先执行，后面的等待。不同文件的操作不受影响，完全并行。

互斥队列以真实文件路径为键（解析 symlink），确保同一文件不因路径别名绕过互斥。队列在操作完成后自动清理，无残留。

### 并行调度

agent 在一个消息中发起多个工具调用时，系统执行策略：

- **读取类操作**（Read、Ls、Grep）：无依赖，全部并行执行
- **写入类操作**（Write、Edit）：同一文件的串行排队，不同文件的并行执行
- **混合操作**（Read 文件 A + Edit 文件 B）：不同文件，互不影响，并行执行
- **混合操作**（Read 文件 A + Edit 文件 A）：Read 先于 Edit 执行——Edit 依赖 Read 的 mtime 做 staleness 检查

### 工具运行时标记

工具通过 ToolFlags 声明自身属性，供调度器和权限引擎参考。以下标记与工具并发调度相关：

- **只读标记**：工具是否只读取数据不产生副作用。用于工具索引渲染（显示"只读"标签）。只读工具同时也是并发安全的，但并发安全标记是独立的判断维度
- **并发分类标记**（`concurrency_class`）：工具是否可在有并行操作时安全调用。只读工具（Read、Ls、Grep）声明并发安全，破坏性工具（Write、Edit、Bash）声明需串行，并发安全但开销大的工具（如 Grep）声明资源消耗型
- **推荐并发上限**（`recommended_concurrency`）：仅资源消耗型工具使用，超出上限的调用排队等待
- **延迟加载标记**：工具的详细描述是否默认不注入 system prompt 一级索引。延迟加载的工具在索引中仅展示名称和危险度标记，LLM 需通过工具发现机制获取完整详情。适用于低频或领域专用工具（技能工具、飞书平台工具、模式与工作流工具等），减少 system prompt 长度

### 提示词引导

系统提示词中明确告知 agent 可以并行调用工具：

- 多个读取操作应合并在一个消息中
- 修改同一文件的多个 Edit 应合并到一次 `edits[]` 调用
- 修改不同文件的操作可以并行发起
- 读取文件 A 和编辑文件 B 可以并行
- 标记为 expensive 的工具（Grep、Bash 等）建议控制并行数量，避免过度消耗系统资源

## 数据流

### 并行读取

```
agent 消息：Read(fileA) + Read(fileB) + Grep(pattern, dirC)
  → 三个操作无依赖，全部并行执行
  → 返回三个结果给 agent
```

### 混合读写（同文件）

```
agent 消息：Read(fileA) + Edit(fileA, edits)
  → Read 先执行（Edit 依赖 Read 的 mtime）
  → Edit 获得 mtime → staleness 检查通过 → 执行修改
  → Read 的结果 + Edit 的结果一起返回给 agent
```

### 修改不同文件

```
agent 消息：Edit(fileA, edits) + Edit(fileB, edits)
  → 两个操作目标文件不同 → 并行执行
  → 各自走独立的 per-file 互斥队列
  → 互不干扰，并行完成
```

### 修改同一文件

```
agent 消息：Edit(fileA, edits1) + Edit(fileA, edits2)（不推荐的用法）
  → edits1 先入 per-file 互斥队列 → 执行 → mtime 变更
  → edits2 从队列中取出 → staleness 检查失败（mtime 已变）
  → 返回错误给 agent："文件已被修改，请重新读取后合并为一次 edits[] 调用"
```

## 模块关系

- **上游**：agent 运行时（解析 LLM 返回的多个 tool call，负责并行调度）
- **下游**：各工具实现（并发安全标记由各工具自行声明）、per-file 互斥队列（Write/Edit 工具调用时入队）
- **无关**：processor_chain（多工具调度在执行层完成，不经过消息处理链路）、IM 适配器（不参与工具调度）
