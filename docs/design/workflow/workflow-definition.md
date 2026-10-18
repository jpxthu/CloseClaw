# Workflow Definition

## 概述

Workflow 定义描述一个多步骤流程的结构化信息，供 Engine 读取并驱动执行。定义嵌入在 SKILL.md 文件的 YAML frontmatter 中，正文部分保留给人/Agent 阅读的原则和注意事项。

## 架构

### 文件位置

Workflow 定义文件按优先级查找，与技能隔离同维度（多 Agent 隔离，详见 [skills](../skills/skill-definition.md)）：

1. Agent 专属目录（`~/.closeclaw/agents/<agent_id>/workflows/`，per-Agent、对其全部 User 共享，与技能隔离同维度，见 [skills 多 Agent 隔离](../../requirements/skills.md)）下的 `workflows/<name>/SKILL.md`
2. 全局 `workflows/<name>/SKILL.md`
3. 系统内置

查找单位为单一定义文件；命中即用、不继续查找下一级。Agent 专属目录为该 Agent 独有、对其全部 User 共享。workflow 定义独立于 skill 目录，不参与 system prompt skill listing。

### 定义结构

一个 workflow 定义在 YAML frontmatter 中包含：

```
Workflow
  ├── id                   // 稳定标识，持久化为 workflow_id
  ├── name                 // 查找键：/workflow <name> 与 workflows/<name>/ 使用
  ├── description          // 注入 workflow context 的描述
  ├── allow_blocked        // 是否允许 Agent 调用 workflow_blocked（可选，默认 false）
  ├── verify_retry_limit   // 验收重试上限（可选，默认 3）
  └── steps: Step[]        // 步骤序列（非空，至少含 Step 0）

Step
  ├── id（从 0 开始的整数）
  ├── name
  ├── allow_blocked        // 覆盖 workflow 级别的 allow_blocked（可选）
  ├── goal                 // 步骤目标（纯文本）
  ├── verify（字符串数组） // 验收清单条目
  ├── jump: JumpQuestion[] // 跳转条件问题
  └── transitions: Transition[]  // 跳转规则

JumpQuestion（统一为枚举选项）
  ├── id（当前 Step 内唯一，作为 workflow_jump 的 answers 键名）
  ├── prompt              // 问题描述
  ├── options             // 选项内部值列表（必填，非空无重复，最多 26）
  └── option_labels       // 选项标签（必填，非空，与 options 一一对应；注入时按 ABCD 顺序渲染）

Transition
  ├── when（可选的 Condition） // 匹配条件，以当前 Step 的 JumpQuestion.id 为键名
  │                             // 格式：{ <jump_id>: <expected_value>, ... }，多条件 AND；未列出的问题不参与匹配
  │                             // expected_value 取 options 中的内部值（非选项字母）
  │                             // 最后一条省略 when 表示 default 兜底；不得写 when: default
  ├── action（goto | reexecute | complete）
  └── target_step         // goto/reexecute 时必填；complete 时省略
```

Workflow 字段直接作为 YAML frontmatter 的顶层键。`id` 为稳定标识（持久化为 workflow_id），`name` 为查找键（`/workflow <name>`、`workflows/<name>/`），`description` 注入 workflow context；三者均非空。`workflow_jump` 的 `answers` 为以 JumpQuestion.id 为键、选项字母为值的对象（如 `{q1: "A", q2: "B"}`）。

### 定义版本

Engine 加载定义时计算定义版本，用于恢复时检测定义变更；版本字段、指纹计算口径与变更判定见 [session-integration.md](session-integration.md)「持久化数据」「定义版本变更」。

### 配置项

allow_blocked（默认 false）：控制 Agent 是否可以在 verifying 阶段调用 workflow_blocked 主动请求阻塞。可在 workflow 级别设置默认值，step 级别覆盖；step 省略时继承 workflow 级值（workflow 级未设置时取 false）。为 true 时，Engine 在验收清单末尾附加 blocked 提示；为 false 时 Agent 调用 workflow_blocked 直接返回错误。

verify_retry_limit（默认 3）：验收重试上限，取正整数（≥1）；0 或负值视为非法定义。Engine 每次注入验收清单后 pending_verify 计数加一。Agent 继续执行未调用 workflow_verify 则待下次验收判定条件满足时由 Engine 重新注入，计数继续累加。计数达到上限 → phase 转为 blocked 并通知 Owner。计数归零条件详见 execution-engine.md「验收重试」。

### 跳转动作

goto：前进到指定步骤，目标 phase 为 executing。
reexecute：重入指定步骤，步骤目标消息注入时附加重新执行提示，目标 phase 为 executing。
complete：Workflow 结束，目标 phase 为 complete。

## 数据流

### 定义加载

Engine 按优先级查找定义文件：

1. Agent 专属目录（`~/.closeclaw/agents/<agent_id>/workflows/`，per-Agent、全 User 共享）下的 `workflows/<name>/SKILL.md`
2. 全局 `workflows/<name>/SKILL.md`
3. 系统内置

三级均未命中 → 返回错误。命中即用，不继续查找下一级；同名定义以最高优先级命中项为准。Engine 解析 YAML frontmatter 得到 Workflow 结构体。定义在每次 workflow 启动时重新加载，执行期间保持不变——变更自下一次启动生效，执行中的 workflow 不受影响（中断续跑恢复视为一次启动，见 session-integration.md）。该生效机制按需求 F1；配置重载的通用机制见 [config §F4](../../requirements/config.md#f4-配置重载)。

### 定义注入

进入 workflow 模式后，Engine 不注入完整定义。Agent 通过以下方式获取步骤信息：

- 步骤目标消息：注入当前步骤的目标描述
- 验收清单：注入当前步骤的验收条目
- 跳转问题：注入当前步骤的跳转问题（含 option_labels 渲染的 ABCD 选项）

workflow context 中注入当前定义文件路径，Agent 如需参考正文可主动读取该文件。SKILL.md 正文中的原则和注意事项不自动注入。Agent 通过 workflow_verify、workflow_jump 等工具将响应回传 Engine。

### 校验流程

create-workflow skill 内置校验脚本，产出 workflow 定义时必须通过。校验覆盖以下项：

- Workflow 字段完整性：id、name、description 非空；steps 非空且至少含 Step 0；verify_retry_limit 为正整数
- 步骤编号合法性：步骤 id 从 0 开始、连续递增，无重复、无遗漏
- 跳转规则合法性：同一 Step 内无重复 when 条件（键值集合完全相同即重复）、必有兜底分支（最后一条省略 when）、goto/reexecute 目标步骤必须存在且 target_step 必填、complete 不得带 target_step；expected_value 必须为对应 options 中的内部值
- 步骤内容完整性：每个步骤的 name、goal 非空
- 验收清单完整性：每个步骤提供非空验收清单
- 选项规范性：options 必填非空无重复、最多 26 项；option_labels 必填非空、与 options 一一对应且最多 26 项；options 内部值不得为单个 A-Z 字母（避免与选项字母混淆）

校验在定义被 Engine 加载时也会再执行一次（防御性）。加载校验失败 → Engine 拒绝启动该 workflow，斜杠指令回复错误、工具调用返回错误，不创建 WorkflowRun。

## 模块关系

### 上游

- **Skills（create-workflow skill）**：Skills 模块提供的 create-workflow skill 产出和修改 workflow 定义，帮助 Agent 按标准格式编写 SKILL.md + YAML frontmatter。内置校验脚本。

### 下游

- **Execution Engine**（同模块子功能）：消费 Workflow 结构体，按 step 定义驱动执行。
- **Workflow Tools**（同模块子功能）：跳转问题的 option_labels 用于渲染工具调用提示。

### 无关

- **普通 Skill**：workflow 定义与技能隔离同维度，但不走 skill 加载和 listing 流程，也不参与 system prompt skill listing。
