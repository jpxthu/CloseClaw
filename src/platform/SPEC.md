# platform 模块规格说明书

## 1. 模块概述

`platform` 模块是 CloseClaw 的多平台 IM 适配层，核心职责是为不同即时通讯平台（飞书、Telegram、Discord、Slack）提供统一的能力抽象和推理模式降级决策。

模块维护各平台的能力矩阵（消息编辑、卡片交互、文件上传、流式输出等），并依据能力配置在运行时决定最优的交互模式。Feishu 作为需要特殊处理的平台，通过 `FeishuAdapter` 实现 Stream→Plan 的降级流程，用交互式卡片模拟流式输出体验。

模块边界：只做平台能力检测和模式决策，不直接对接外部 API，不处理具体的消息收发逻辑。

---

## 2. 公开接口

### 常量

| 标识符 | 描述 |
|--------|------|
| `PLATFORM_FEISHU` | 飞书平台标识 |
| `PLATFORM_TELEGRAM` | Telegram 平台标识 |
| `PLATFORM_DISCORD` | Discord 平台标识 |
| `PLATFORM_SLACK` | Slack 平台标识 |
| `FEISHU_STREAM_FALLBACK_STEPS` | Feishu Stream→Plan 降级的 4 步流程定义（定义于 `feishu/fallback.rs`） |
| `COMPLEXITY_INDICATORS` | 高复杂度任务的 6 个关键词指标（定义于 `feishu/complexity.rs`） |

---

### 构造（Construction）

| 接口 | 描述 |
|------|------|
| `PlatformCapabilityService::new()` | 构造能力服务，注册已知平台的能力矩阵 |
| `FeishuAdapter::new(...)` | 构造飞书适配器，注入能力服务、卡片服务、消息服务 |
| `FallbackResult` | Fallback 执行结果结构体，含 initial/card/final 三个 message_id |
| `ModeDecisionContext::new(session_id)` | 构造模式决策上下文（Builder 起点） |

---

### 配置（Configuration）

| 接口 | 描述 |
|------|------|
| `default_capabilities()` | 返回未知平台的默认能力配置（全不支持） |
| `ReasoningMode` | 从 `session::persistence` 重新导出的推理模式枚举（Stream/Plan/Direct/Hidden） |
| `ModeDecisionContext::with_requested_mode(mode)` | 设置本次决策请求的推理模式 |
| `ModeDecisionContext::with_metadata(key, value)` | 向上下文附加键值元数据 |

---

### 主操作（Primary Operations）

| 接口 | 描述 |
|------|------|
| `PlatformCapabilityService::get_capabilities(platform)` | 查询指定平台的能力配置 |
| `PlatformCapabilityService::get_fallback_mode(platform, mode)` | 获取某平台在某模式下应降级到的目标模式 |
| `FeishuAdapter::execute_fallback(intent)` | 执行完整的 Stream→Plan 降级流程 |
| `FeishuAdapter::handle_mode_switch(event)` | 处理模式切换事件，判断是否需要降级并触发降级流程 |
| `build_initial_sections(goal)` | 根据目标构建 Plan 卡片的初始区段列表 |
| `default_plan_card_config(goal)` | 构建带进度的 Plan 卡片默认配置 |
| `get_fallback_steps(mode)` | 获取指定模式对应的降级步骤序列 |
| `run_streaming_with_card_update(...)` | 模拟流式输出，逐步更新卡片各区段 |
| `update_card_content(...)` | 在流式过程中更新卡片指定区段的内容 |
| `SectionUpdate` | 卡片区段增量更新结构 |
| `CardHandle` | 卡片创建后返回的句柄（仅含 message_id） |
| `FeishuAdapterError::CardService(String)` | 卡片服务错误变体（定义于 error.rs） |
| `HighComplexityConfig` | 高复杂度任务的展示增强配置 |
| `is_high_complexity(intent)` | 判断用户意图描述的任务是否属于高复杂度 |
| `get_high_complexity_config()` | 获取高复杂度任务的展示增强配置 |
| `FallbackAction` | Fallback 步骤动作类型枚举（SendInitialMessage/CreateCard/UpdateCard/SendFinal） |
| `FallbackStep` | Fallback 单个步骤结构体（含序号、动作、静态描述、persist 标志） |
| `FallbackStep::content_string()` | 将步骤的静态内容转换为 String |
| `CardService` | 卡片服务抽象 trait，含 `create_card`、`update_section`、`update_progress`、`mark_step_complete`、`update_card` 五个异步方法 |

---

### 查询（Query）

| 接口 | 描述 |
|------|------|
| `PlatformCapabilityService::supports_mode(platform, mode)` | 检查平台是否支持某推理模式 |
| `PlatformCapabilityService::supports_mode_fully(platform, mode)` | 检查平台是否完整支持某推理模式 |
| `FeishuAdapter::should_fallback(mode)` | 判断某模式在飞书平台是否需要降级 |
| `FeishuAdapter::get_fallback_mode()` | 获取飞书平台 Stream 模式对应的降级目标 |
| `FeishuAdapter::is_fallback_enabled()` | 检查飞书降级流程是否已启用 |

---

## 3. 架构与结构

### 3.1 子模块划分

```
platform/
├── mod.rs              # 模块入口，导出常量、公共类型、default_capabilities
├── capabilities.rs     # 能力矩阵定义与服务
└── feishu/
    ├── mod.rs          # 导出 FeishuAdapter、FallbackResult、CardService
    ├── adapter.rs      # FeishuAdapter 核心：降级决策与流程编排
    ├── card.rs         # Plan 卡片结构：PlanCardConfig、PlanSection、StepStatus
    ├── card_updater.rs # CardService trait：卡片创建与更新操作抽象
    ├── fallback.rs     # 降级步骤定义：FallbackAction、FallbackStep
    ├── updater.rs      # 流式更新逻辑：模拟流式输出驱动的卡片刷新
    ├── complexity.rs   # 高复杂度任务检测：指标词、is_high_complexity
    └── error.rs        # 错误类型：FeishuAdapterError（9 个变体，含 Io、Serialization）
```

**`capabilities.rs`** 定义能力枚举 `CapabilityLevel`、`FileUploadCapability`，结构体 `PlatformCapabilities`，以及查询服务 `PlatformCapabilityService` 和决策上下文 `ModeDecisionContext`。

**`feishu/`** 是 Feishu 平台的具体实现子包，`adapter.rs` 是入口，按需组合 card、card_updater、fallback、updater、complexity 四个子模块。

---

### 3.2 数据流

```
用户发起 ModeSwitchEvent
       │
       ▼
FeishuAdapter::handle_mode_switch(event)
       │
       ├─→ should_fallback(mode) ──(No)──→ 直接返回 None
       │
       └─→ (Yes) execute_fallback(intent)
                    │
                    ├─1. send_initial_message()      → 发送 "🔍 进入深度分析模式..."
                    │
                    ├─2. create_plan_card(goal)      → Feishu CardService::create_card
                    │                                   产出 CardHandle { message_id }
                    │
                    ├─3. run_streaming_with_card_update()
                    │       │
                    │       ├─ update_progress(1/N)
                    │       ├─ 对每个 section:
                    │       │     update_section(idx, Active)
                    │       │     mark_step_complete(n)
                    │       └─ update_progress(N/N)
                    │
                    └─4. send_final_message()        → 发送 "✅ 分析完成"
                    
       ▼
返回 FallbackResult { initial_message_id, card_message_id, final_message_id }
```

降级流程中，卡片内容更新由 `CardService` trait 的实现层完成。

---

### 3.3 跨模块格式

**意图结构（Intent）**：由 `session::events::ModeSwitchEvent` 传入，包含 `target_mode`、`requested_mode`、`user_intent`（含 `parsed_goal`），用于驱动降级流程和高复杂度判断。

**卡片配置（Card）**：在子模块间传递时使用 `PlanCardConfig`（卡片整体配置）和 `CardHandle`（仅含 `message_id`，由 card_updater 创建后沿流程传递）。

**区段更新（Update）**：流式更新时通过 `SectionUpdate` 结构传递增量修改（`title`、`content`、`status` 均为 `Option`）。

**错误类型**：所有 Feishu 子模块的错误统一为 `FeishuAdapterError` 枚举，向上对 adapter 暴露。

---

*最后更新：2026-04-14*
