# renderer 子模块规格说明书

> 本文件按 SPEC_CONVENTION.md v3 标准编写，描述模块的实际行为，以代码为准。

## 1. 模块概述

`renderer` 子模块定义 Rendering Layer 的基础设施，将 LLM 输出的 markdown 内容渲染为平台特定的格式。核心抽象为 `Renderer` trait 和 `RenderedOutput` 类型。

第一个平台实现 `FeishuRenderer` 将 markdown（及可选的 DSL 指令）渲染为飞书交互卡片或纯文本消息。渲染决策完全由内容本身决定：空内容或纯文本返回 text 类型，含富格式/换行/DSL 时返回 interactive 类型。

`ProcessorChainConfig` 新增 `renderer` 配置字段，`ProcessorChainLoader::load()` 返回 `Option<Arc<dyn Renderer>>`，与 Processor 链解耦。

**文件**：
- `src/renderer/mod.rs` — `RenderedOutput`、`Renderer` trait
- `src/renderer/feishu.rs` — `FeishuRenderer` 实现
- `src/processor_chain/loader.rs` — `ProcessorChainConfig` renderer 字段及 loader 集成

## 2. 公开接口

### 2.1 Renderer trait

| 接口 | 功能 |
|------|------|
| `Renderer::platform` | 返回平台名称，如 `"feishu"` |
| `Renderer::render` | 将 markdown 内容渲染为 `RenderedOutput`；`dsl_result` 为 `None` 时正常渲染不报错 |

> `Renderer` trait bound: `Send + Sync`（允许跨 async context 共享）

### 2.2 RenderedOutput

| 字段 | 说明 |
|------|------|
| `msg_type: String` | 消息类型，`"text"` 或 `"interactive"` |
| `payload: serde_json::Value` | 平台相关 payload JSON |

### 2.3 FeishuRenderer

| 接口 | 功能 |
|------|------|
| `FeishuRenderer::new` | 创建实例 |
| `Renderer` impl | 实现 `platform() -> "feishu"` 和 `render()` |

渲染规则（`render()` 决策逻辑）：

| 输入 | 输出 |
|------|------|
| 空 content | `msg_type: "text"`，空文本 JSON |
| 纯文本（无 markdown 富格式、无换行、无 `#`、无 DSL） | `msg_type: "text"` |
| 含 `# Title`（标题后有正文内容） | 提取为 `header.title`（template `"blue"`），正文部分按内容类型决定是否 card |
| 含 `---` | 转为 `hr` element |
| 含 DSL 按钮指令 | 渲染为 `action` element；第一个按钮为 `primary`，其余为 `default` |
| 含换行/富格式（`**`、`__`、`*`、`_`、`\``、`[](`） | `msg_type: "interactive"` |

### 2.4 ProcessorChainLoader 配置集成

| 配置类型 | 说明 |
|------|------|
| `ProcessorChainConfig.renderer: Option<RendererConfig>` | YAML 中可选字段，默认为 `None` |
| `RendererConfig::Feishu` | 飞书平台 renderer，serde tag `"type": "feishu"` |

| 接口 | 功能 |
|------|------|
| `ProcessorChainLoader::load` | 返回 `(ProcessorRegistry, Option<Arc<dyn Renderer>>)`；无 renderer 配置时返回 `None` |

## 3. 架构/结构

### 3.1 子模块划分

- `renderer/mod.rs` — 核心 trait 和类型定义，`pub mod feishu`
- `renderer/feishu.rs` — `FeishuRenderer` 实现，自含 Card 相关类型（`CardPayload`、`Card`、`CardHeader`、`CardElement`、`CardAction`、`CardText`）

> 注：FeishuRenderer 卡片类型独立定义于 `feishu.rs` 内，未从 `MarkdownToCard` 复用。

### 3.2 数据流

```
LLM markdown 输出
    │
    ▼
FeishuRenderer::render(content, dsl_result)
    │
    ├─ 空 content → build_text("")
    │
    ├─ 不需要 card（纯文本无格式）→ build_text(content)
    │
    └─ 需要 card
         ├─ extract_header → 提取 # Title 为 header
         ├─ to_elements → 换行分割，--- → hr element
         ├─ render_buttons → DSL 按钮转为 action element
         └─ build_card → RenderedOutput { msg_type: "interactive", payload }
    │
    ▼
RenderedOutput { msg_type, payload }
```

### 3.3 ProcessorChainLoader 中的集成

```
ProcessorChainConfig { inbound, outbound, renderer: Option<RendererConfig> }
    │
    ▼
ProcessorChainLoader::load()
    ├─ 构建 ProcessorRegistry（inbound + outbound processors）
    └─ build_renderer(config.renderer.as_ref())
         ├─ Some(RendererConfig::Feishu) → Some(Arc::new(FeishuRenderer::new()))
         └─ None → None
    │
    ▼
(ProcessorRegistry, Option<Arc<dyn Renderer>>)
```

### 3.4 FeishuRenderer 卡片结构

```json
{
  "msg_type": "interactive",
  "card": {
    "header": { "title": "...", "template": "blue" },
    "elements": [
      { "tag": "markdown", "content": "..." },
      { "tag": "hr" },
      { "tag": "action", "actions": [{ "tag": "button", "text": { "tag": "plain_text", "content": "..." }, "type": "primary" }] }
    ]
  }
}
```