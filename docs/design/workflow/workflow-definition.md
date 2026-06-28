# Workflow Definition

## 概述

Workflow 定义描述一个多步骤流程的结构化信息，供 Engine 读取并驱动执行。定义嵌入在 SKILL.md 文件的 YAML frontmatter 中，正文部分保留给人/Agent 阅读的原则和注意事项。

## 架构

### 文件位置

Workflow 定义文件放在 agent workspace 或全局目录下，目录结构效仿 skill：

```
workspace/
├── skills/          ← 普通 skill
└── workflows/       ← workflow skill（含 YAML frontmatter）
    └── design-doc-modify/
        ├── SKILL.md        ← 正文 + workflow frontmatter
        └── references/     ← 分身指令等
```

优先级查找：agent workspace > `.closeclaw/workflows/` > 内置。与 skill 目录同构但独立——workflow 不参与 system prompt skill listing。

### 定义结构

一个 workflow 定义包含：

```
Workflow
  ├── id, name, description
  ├── step_data_schema    // 跨步骤共享数据的字段定义（键值对，由 Engine 在步骤间维护）
  └── verify_retry_limit  // 验证重试上限（默认 3，--optional）
  └── steps: Step[]       // 步骤序列

Step
  ├── id（从 0 开始的整数）
  ├── name
  ├── type: action | blocking
  ├── goal                 // 步骤目标（纯文本）
  ├── verify: string[]     // 验收清单条目（Agent 自查，Engine 不验证真伪）
  ├── jump: JumpQuestion[] // 跳转条件问题
  └── transitions: Transition[]  // 跳转规则

JumpQuestion
  ├── id（键名，作为 workflow_jump 的参数键名）
  ├── prompt              // 问题描述
  ├── type: boolean | enum | string
  ├── options             // enum 类型的选项值（boolean 和 string 类型忽略此字段）
  └── option_labels       // 注入时渲染为 ABCD 的显示标签

Transition
  ├── when: Condition     // 匹配条件，以 JumpQuestion.id 为键名匹配答案
  │                         // 格式：{ <jump_id>: <expected_value>, ... }，多个条件为 AND
  ├── action: goto | reexecute | complete
  └── target_step         // goto/reexecute 时的目标步骤，最后一条可用 default 替代 when
```

### 步骤类型

`action`：标准步骤，Agent 执行 → Engine 验证 → 跳转。

`blocking`：阻塞步骤，需要 owner 输入后才能继续。Engine 在 goal 注入后将步骤状态标记为 blocked，等 owner 输入到达后进入 verify 流程。

### 跳转动作

`goto(N)`：前进到指定步骤，清空 step_data。

`reexecute(N)`：重入指定步骤，保留 step_data，标记重跑。goal 注入时附加"重新执行"提示。

`complete`：Workflow 结束。

## 数据流

### 定义加载

```
/workflow <name> 或 workflow_start({name})
  ↓
Engine 按优先级查找定义文件
  ├─ agent workspace/workflows/<name>/SKILL.md
  ├─ .closeclaw/workflows/<name>/SKILL.md
  └─ 内置 workflow/<name>/SKILL.md
  ↓
Engine 解析 YAML frontmatter → Workflow 结构体
  ↓
Engine 缓存定义（session 生命周期内不变）
```

### 定义注入

进入 workflow 模式后，Engine 不注入完整定义。Agent 通过以下方式获取步骤信息：

- goal 消息：注入当前步骤的目标描述
- verify 消息：注入验收清单
- jump 消息：注入跳转问题（含 option_labels 渲染的 ABCD 选项）

SKILL.md 正文中的原则和注意事项不自动注入——Agent 如需参考，应主动读取文件。这与普通 skill 的加载方式一致。

### 校验流程

create-workflow skill 内置校验脚本，产出 workflow 定义时必须通过。校验在定义被 Engine 加载时也会再执行一次（防御性）。

## 模块关系

### 上游

- **create-workflow skill**：产出 workflow 定义的 skill，帮助 Agent 按标准格式编写 SKILL.md + YAML frontmatter。内置校验脚本。

### 下游

- **Execution Engine**：消费 Workflow 结构体，按 step 定义驱动执行。
- **Workflow Tools**：jump 问题的 option_labels 用于渲染工具调用提示。

### 无关

- **普通 Skill**（无调用关系）：workflow 定义文件复用 skill 目录结构，但不走 skill 加载和 listing 流程。
