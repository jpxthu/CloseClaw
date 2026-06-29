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

优先级查找：agent workspace/workflows/ > .closeclaw/workflows/ > 内置。与 skill 目录同构但独立——workflow 不参与 system prompt skill listing。

### 定义结构

一个 workflow 定义在 YAML frontmatter 中包含：

```
Workflow
  ├── id、name、description
  ├── allow_blocked        // 是否允许 Agent 调用 workflow_blocked（可选，默认 false）
  ├── verify_retry_limit   // 验证重试上限（可选，默认 3）
  ├── step_data_schema     // 跨步骤共享数据的字段声明，格式为 { 字段名: 类型 }
  └── steps: Step[]        // 步骤序列

Step
  ├── id（从 0 开始的整数）
  ├── name
  ├── allow_blocked        // 覆盖 workflow 级别的 allow_blocked（可选）
  ├── goal                 // 步骤目标（纯文本）
  ├── verify（字符串数组） // 验收清单条目
  ├── jump: JumpQuestion[] // 跳转条件问题
  └── transitions: Transition[]  // 跳转规则

JumpQuestion
  ├── id（键名，作为 workflow_jump 的参数键名）
  ├── prompt              // 问题描述
  ├── type（boolean | enum）
  ├── options             // enum 类型必填，选项值列表（boolean 类型忽略）
  └── option_labels       // 字符串数组，与 options 一一对应，注入时按 ABCD 顺序渲染

Transition
  ├── when（可选的 Condition） // 匹配条件，以 JumpQuestion.id 为键名
  │                             // 格式：{ <jump_id>: <expected_value>, ... }，多条件 AND
  │                             // boolean 类型用 YAML 原生布尔值 true/false
  │                             // 最后一条省略 when 表示 default 兜底
  ├── action（goto | reexecute | complete）
  └── target_step         // goto/reexecute 时的目标步骤
```

### 配置项

allow_blocked（默认 false）：控制 Agent 是否可以在 verify 阶段调用 workflow_blocked 主动请求阻塞。可在 workflow 级别设置默认值，step 级别覆盖。为 true 时，Engine 在 verify 消息末尾附加 blocked 提示；为 false 时 Agent 调用 workflow_blocked 直接返回错误。

verify_retry_limit（默认 3）：验证重试上限。Engine 每次注入验收清单后 pending_verify 计数加一。Agent 继续执行未调 verify 则等下次 idle 重新注入，计数继续累加。计数超过上限 → phase 转为 blocked 并通知 owner。Agent 调用 workflow_verify、goto 到新步骤、reexecute 重入步骤、或 owner 解除 blocked 后计数归零（详见 execution-engine.md）。

### 跳转动作

goto：前进到指定步骤，清空 step_data，目标 phase 为 executing。
reexecute：重入指定步骤，保留 step_data，goal 注入时附加重新执行提示，目标 phase 为 executing。
complete：Workflow 结束，目标 phase 为 complete。

## 数据流

### 定义加载

Engine 按优先级查找定义文件：

1. agent workspace/workflows/<name>/SKILL.md
2. .closeclaw/workflows/<name>/SKILL.md
3. 内置 workflow/<name>/SKILL.md

三级均未命中 → 返回错误。命中即用，不继续查找下一级。Engine 解析 YAML frontmatter 得到 Workflow 结构体，缓存定义（session 生命周期内不变）。

### 定义注入

进入 workflow 模式后，Engine 不注入完整定义。Agent 通过以下方式获取步骤信息：

- goal 消息：注入当前步骤的目标描述
- verify 消息：注入验收清单
- jump 消息：注入跳转问题（含 option_labels 渲染的 ABCD 选项）

SKILL.md 正文中的原则和注意事项不自动注入——Agent 如需参考，应主动读取文件。Agent 通过 workflow_verify、workflow_jump 等工具将响应回传 Engine。

### 校验流程

create-workflow skill 内置校验脚本，产出 workflow 定义时必须通过。校验在定义被 Engine 加载时也会再执行一次（防御性）。

## 模块关系

### 上游

- **create-workflow skill**：产出 workflow 定义的 skill，帮助 Agent 按标准格式编写 SKILL.md + YAML frontmatter。内置校验脚本。

### 下游

- **Execution Engine**（同模块）：消费 Workflow 结构体，按 step 定义驱动执行。
- **Workflow Tools**（同模块）：jump 问题的 option_labels 用于渲染工具调用提示。

### 无关

- **普通 Skill**：workflow 定义文件复用 skill 目录结构，但不走 skill 加载和 listing 流程。
