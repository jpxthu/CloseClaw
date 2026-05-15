# 代码差异记录

> 本目录记录 processor_chain 设计文档与实际代码实现之间的差异。

---

## SPEC.md 与 Renderer 架构不一致

**文件**：`SPEC.md`
**发现时间**：设计文档 review 过程中

### 差异描述

SPEC.md 中将 `MarkdownToCard` 描述为出站处理器，这是旧架构。

目标架构中，出站渲染职责已迁移至 `Renderer` trait：
- `src/renderer/mod.rs` — `Renderer` trait 定义
- `src/renderer/feishu.rs` — `FeishuRenderer` 实现
- `src/card/renderer.rs` — card 模块内的飞书渲染（另一套 renderer，职责不同）

`MarkdownToCard` 可能在某些遗留路径中仍有引用，需要清理。

### 影响范围

需确认：
1. `MarkdownToCard` 当前是否还有实际调用路径
2. SPEC.md 中哪些章节描述的是旧架构，需要更新
3. 两套 renderer（`src/renderer/` vs `src/card/renderer.rs`）的边界是否清晰

### 状态

🟡 待处理 — 需另开 session 梳理并更新 SPEC