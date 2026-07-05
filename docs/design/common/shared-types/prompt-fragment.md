# FragmentContext

## 概述

FragmentContext 是 PromptFragmentProvider 片段生成时的输入上下文，由 System Prompt Builder 构建后传递给各 Provider。PromptFragment 是单个 Provider 产出的系统 prompt 静态层片段。

> **本文档定义的 FragmentContext、PromptFragment、BootstrapMode、SectionType 在 common crate 中实现。引用本模块的下游文档通过这些链接引用这些类型定义，不在自身模块的文档或代码中重复实现。**

## 架构

### FragmentContext

FragmentContext 是 PromptFragmentProvider 片段生成时的输入上下文，由 System Prompt Builder 构建后传递给各 Provider。

| 字段 | 类型 | 说明 |
|------|------|------|
| `agent_id` | string | Agent 标识。Skills 按此过滤可见 skill |
| `bootstrap_mode` | enum | BootstrapMode::Minimal（精简模式）或 BootstrapMode::Full（完整模式），Bootstrap 按此选择文件集合 |
| `workdir` | string | agent 工作目录路径，Bootstrap 按此查找 bootstrap 文件 |

### BootstrapMode

BootstrapMode 是引导模式的枚举，控制 Bootstrap 文件的选择范围。

| 值 | 说明 |
|----|------|
| Minimal | 精简模式，仅加载必要文件 |
| Full | 完整模式，加载所有可用文件 |

### PromptFragment

PromptFragment 是单个 PromptFragmentProvider 产出的静态层片段。

| 字段 | 类型 | 说明 |
|------|------|------|
| `section_title` | string | Section 标题，如 `## AGENTS.md`、`## Available Skills` |
| `section_type` | enum | Section 类型：bootstrap 文件、工具列表、skill 清单、长期记忆 |
| `content` | string | 渲染完成的文本内容 |

### SectionType

SectionType 是 PromptFragment 的 section 类型枚举，用于 Builder 按类型管理片段的排序和汇聚。

| 值 | 说明 |
|----|------|
| bootstrap 文件 | 来自 bootstrap 目录的文件内容 |
| 工具列表 | Agent 可用的工具描述清单 |
| skill 清单 | Agent 可用的 skill 描述清单 |
| 长期记忆 | 跨 session 持久化的长期记忆片段 |

## 数据流

FragmentContext 和 PromptFragment 的流动嵌入在 system prompt 静态层的构建流程中：

```
SessionManager 触发构建
  ↓
System Prompt Builder 构建 FragmentContext（agent_id + bootstrap_mode + workdir）
  ↓
遍历已注册的 PromptFragmentProvider → 传入 FragmentContext → 各 Provider 产出 PromptFragment
  ↓
按优先级拼接所有 PromptFragment.content
  ↓
写入 ConversationSession 的 system prompt 字段
```

FragmentContext 由 Builder 一次性构建，所有 Provider 共享同一上下文。PromptFragment 由各 Provider 独立产出，生命周期止于 Builder 完成拼接。

## 模块关系

### FragmentContext

- **生产者**：system_prompt 模块（System Prompt Builder 构建）
- **消费者**：所有 PromptFragmentProvider 实现者（system_prompt / tools / skills / memory）
- **无关**：LLM Provider（不接触 FragmentContext）、Processor Chain（不参与 system prompt 构建）

### PromptFragment

- **生产者**：所有 PromptFragmentProvider 实现者（system_prompt / tools / skills / memory）
- **消费者**：system_prompt 模块（System Prompt Builder 收集所有 Fragment 并按序拼接）
- **无关**：LLM Provider（不接触 PromptFragment，消费的是拼接后的最终 system prompt 文本）、Session（Builder 写入 system prompt 字段，Session 不直接操作 PromptFragment）

### BootstrapMode

- **生产者**：system_prompt 模块（System Prompt Builder 在构建 FragmentContext 时设置 bootstrap_mode 字段）
- **消费者**：Bootstrap 模块（读取 bootstrap_mode 选择加载的文件集合）
- **无关**：LLM Provider（不感知 BootstrapMode）、Processor Chain（不参与 system prompt 构建）

### SectionType

- **生产者**：各 PromptFragmentProvider 实现者（生成 PromptFragment 时指定 section_type）
- **消费者**：system_prompt 模块（System Prompt Builder 按 section_type 对片段做排序和汇聚）
- **无关**：LLM Provider（不感知 SectionType）、Processor Chain（不参与 system prompt 构建）
