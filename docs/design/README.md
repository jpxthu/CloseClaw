# CloseClaw 设计文档索引

## 概述

本目录存放 CloseClaw 各模块的设计文档，按编号顺序排列（倒序）。

设计文档描述**设计决策、历史上下文、为什么这么做**，区别于模块规格书（`src/<模块>/SPEC.md`）描述"模块是什么"。

## 文档顺序与覆盖关系

| 编号 | 文件名 | 状态 | 覆盖关系 |
|------|--------|------|----------|
| 43 | `43-new-session-injection-chain.md` | ✅ 最新 |  |
| 42 | `42-permissions.md` | ✅ 最新 | v2，替代 03 |
| 41 | `41-bash-tool.md` | ✅ 最新 |  |
| 40 | `40-tools-prompt-injection.md` | ✅ 最新 |  |
| 39 | `39-llm-config-wizard.md` | ✅ 最新 |  |
| 38 | `38-message-processor-rendering-layer.md` | ✅ 最新 | 替代 27 部分 |
| 37 | `37-llm-multi-provider-arch.md` | ✅ 最新 |  |
| 36 | `36-llm-session-enhancements.md` | ✅ 最新 |  |
| 35 | `35-llm-model-discovery-wizard.md` | ✅ 最新 |  |
| 34 | `34-compact-process-integration.md` | ✅ 最新 | 替代 34-session-compaction |
| 34 | `34-session-compaction.md` | ⚠️ 废弃 | 被 34-compact-process-integration 替代 |
| 33 | `33-session-archive-sweeper-scheduling.md` | ✅ 最新 |  |
| 32 | `32-skill-listing-injection.md` | ✅ 最新 |  |
| 31 | `31-llm-outbound-format-conversion.md` | ✅ 最新 |  |
| 30 | `30-skill-system-redesign.md` | ✅ 最新 |  |
| 29 | `29-bootstrap-unified-loader.md` | ✅ 最新 |  |
| 28 | `28-tool-system-hierarchy.md` | ✅ 最新 |  |
| 27 | `27-message-processor-architecture.md` | ⚠️ 部分废弃 | 部分被 38 替代 |
| 26 | `26-subagent-independent-config.md` | ✅ 最新 |  |
| 25 | `25-plan-mode.md` | ✅ 最新 | v2 比 v1 更完整 |
| 25 | `25-plan-mode-v2.md` | ✅ 最新 | 最终版 |
| 24 | `24-config-file-split.md` | ✅ 最新 |  |
| 23 | `23-session-archive-lifecycle.md` | ✅ 最新 |  |
| 22 | `22-gradual-rollout.md` | ✅ 最新 |  |
| 21 | `21-monitoring-alerting.md` | ✅ 最新 |  |
| 20 | `20-eda-scheduler.md` | ✅ 最新 |  |
| 19 | `19-braino-deep-integration.md` | ✅ 最新 |  |
| 18 | `18-smart-mode-recommend.md` | ✅ 最新 |  |
| 17 | `17-mindmap-export.md` | ✅ 最新 |  |
| 16 | `16-openclaw-config-hot-reload.md` | ✅ 最新 |  |
| 15 | `15-test-process-standard.md` | ✅ 最新 |  |
| 14 | `14-markdown-code-render.md` | ✅ 最新 |  |
| 13 | `13-streaming-line-render.md` | ✅ 最新 |  |
| 12 | `12-compaction-marker-compression.md` | ✅ 最新 |  |
| 11 | `11-compaction-bootstrap-protection-implementation.md` | ✅ 最新 |  |
| 10 | `10-bootstrap-loading-and-session-filtering.md` | ✅ 最新 |  |
| 09 | `09-subagent-coordination-protocol.md` | ✅ 最新 |  |
| 08 | `08-tool-prompt-dynamic-generation.md` | ✅ 最新 |  |
| 07 | `07-multi-provider-cache-adapter.md` | ✅ 最新 |  |
| 06 | `06-agent-type-config.md` | ✅ 最新 |  |
| 05 | `05-slash-command.md` | ✅ 最新 |  |
| 04 | `04-heartbeat-dedup.md` | ✅ 最新 |  |
| 03 | `03-permissions.md` | ❌ 废弃 | 被 42 替代 |
| 02 | `02-tools-keywords.md` | ✅ 最新 |  |
| 01 | `01-system-prompt-sectioning.md` | ✅ 最新 |  |

## 模块对应关系

> 渐进映射中，待完成

| 模块 | 设计文档 | SPEC.md |
|------|----------|---------|
| `src/agent/` | - | - |
| `src/audit/` | - | - |
| `src/card/` | - | - |
| `src/chat/` | - | - |
| `src/cli/` | - | - |
| `src/config/` | - | - |
| `src/daemon/` | - | - |
| `src/gateway/` | - | - |
| `src/im/` | - | - |
| `src/llm/` | - | - |
| `src/mode/` | - | - |
| `src/permission/` | - | - |
| `src/platform/` | - | - |
| `src/processor_chain/` | - | - |
| `src/renderer/` | - | - |
| `src/session/` | - | - |
| `src/skills/` | - | - |
| `src/system_prompt/` | - | - |
| `src/tools/` | - | - |

## 文档来源

原始设计文档来自 `~/.openclaw/agents/braino/workspace/design/`。