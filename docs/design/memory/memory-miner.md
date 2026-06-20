# memory-miner

## 概述

会话 transcript 产生后，用独立 session 进行记忆挖掘，提取值得长期保留的信息，产出结构化记忆条目。触发方式有两种：即时 hook（sub-agent session 结束）和定时任务（archived 会话扫描）。

## 架构

memory-miner 是独立于对话主 session 的异步任务。不与用户交互，不阻塞主流程。

**两种触发机制**：

```
触发 1：Sub-agent session 结束
  session 结束 hook → 即时触发 mining
  适用于生命周期明确的 session（子 agent 完成/失败后）

触发 2：Daemon DreamingScheduler 定时任务
  定时扫 archived 且 mined=false 的会话 → 触发 mining
  适用于 owner 会话等无明确结束点的会话
  （DreamingScheduler 的整体调度顺序：先 dreaming 后 mining，详见 README）
```

**挖掘流程**（两种触发共用）：
- 输入：完整会话 transcript（脱敏后）+ 已有记忆条目（避免重复）+ 近期日常记忆（避免重复）
- 模型：独立 session，使用独立 prompt 进行挖掘
- 输出：结构化记忆条目，写入 memory store
- 完成后：标记会话 `mined=true`

**挖掘原则**：
- 只提取 durable 信息（决策、偏好、纠正、教训），不记录临时上下文细节
- 与已有条目去重：相同语义的信息不重复记录
- 与近期日常记忆去重：已在其他条目中记录的内容不再产出

## 数据流

```
输入                   处理                    输出
─────                 ─────                  ─────
会话 transcript  ─→   独立 mining session   ─→  结构化记忆条目
已有记忆条目    ─→   （专用 prompt）              · 类别
近期日常记忆    ─→   去重 + 提取                 · 正文
                                                · 时间戳
                                                · 来源会话
                                                │
                                                ▼
                                          标记会话 mined=true
                                          写入 memory store
                                          （Markdown 多文件）
```

## 模块关系

- **上游**：
  - session 模块：sub-agent session 结束时通过 hook 触发，提供完整 transcript
  - daemon 模块：DreamingScheduler 定时任务触发，扫描 archived 会话
  - memory store：提供已有记忆条目和近期日常记忆，用于去重

- **下游**：
  - dreaming：消费结构化条目进行定期升格
  - active-searcher：搜索索引覆盖 miner 写入的条目
