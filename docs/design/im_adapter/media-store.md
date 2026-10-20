# 媒体存储

## 概述

- 关联需求：[requirements/im_adapter.md §F9](../../requirements/im_adapter.md)
- 媒体存储是 IM Adapter 的通用子功能（平台无关），按需求当前**暂不设计**：需求 §F9 与 [gateway §F3](../../requirements/gateway.md) 均标注「按 Owner 要求暂不设计」，本节内容不作为当前设计约束。因此本子功能当前**不实现**任何媒体能力，也不预留实现契约；启用前需先解冻需求并重新设计。

## 架构

当前不实现以下能力：

- 入站媒体的下载、落盘与文件命名/净化
- NormalizedMessage 的 media_refs / unavailable_media 填充
- Agent 上下文中的媒体形态决策（图片内容、媒体引用、不可得处理）
- 出站媒体的读取、来源白名单校验、保留副本与上传发送
- 媒体生命周期清理（保留期清理、跟随 Session 清理）
- [config media.json](../config/README.md) 的媒体配置项

非文本消息（图片、文件、音频）的媒体获取、落盘与进入对话同样不在当前实现范围；post 消息的文本展开与 `[图片]` 占位符保留属文本解析，仍正常实现（见 [飞书插件](platforms/feishu.md)）。

## 数据流

不实现。入站媒体不落盘、不填充媒体引用、不进入 Agent 上下文；出站不读取或发送媒体文件。

## 模块关系

- **上游**：无（当前不实现，不消费任何模块数据）
- **下游**：无（当前不实现，不向任何模块产出数据）
- **共享类型 / 核心 trait**：[common/shared-types](../common/shared-types.md) 的 media_refs / unavailable_media / MediaRef 与 [common/core-traits](../common/core-traits.md) 的 MediaStoreAccess 均随媒体能力暂缓，当前不产生、不填充、不实现
- **无关**：Processor Chain（媒体引用不入链）、Permission（媒体能力未启用，不涉及权限判断）
