# Team Memory — 团队共享知识空间

## 背景与问题

**现状**：
- 每个 Agent 有独立的 `memory/` 文件，但只属于自己
- 脑暴虾做完设计后，计划虾要翻聊天记录才能拿到上下文
- 决策分散在各个 Agent 的记忆里，没有集中记录

**需求**：
- 所有 Agent 都能读写的共享知识空间
- 包含：设计决策、技术选型结论、项目状态、人物关系
- 人类（Owner）能在飞书直接查看/编辑

## 方案选型

### 方案 A：飞书 Wiki 作为主存储

Agent 通过 Feishu API 读写 Wiki 文档，人类直接在飞书编辑。

| 优点 | 缺点 |
|------|------|
| 人类可直接查看/编辑，无需额外 UI | Wiki 结构是树状的，不适合 Agent 高频写入 |
| 飞书自带权限管理 | 需要处理并发写入冲突 |
| 天然支持多端访问 | Agent 每次写入都要调 API，有 rate limit 风险 |

### 方案 B：本地 Markdown 文件 + 飞书只读镜像

共享知识存在 `~/.closeclaw/team-memory/`，Agent 可直接读写。飞书侧通过 Bot 定期同步内容为只读文档。

| 优点 | 缺点 |
|------|------|
| Agent 读写无 API 限制，本地文件操作简单 | 需要解决多 Agent 并发写入问题 |
| 可 Git 版本化 | 飞书镜像是单向的，人类写入不同步回来 |
| 可接入 semantic search | 镜像同步有时延 |

### 方案 C：飞书多维表格（Bitable）作为结构化存储

用 Bitable 做知识库，每条记录是独立的 fact/decision，支持多维度筛选。

| 优点 | 缺点 |
|------|------|
| 结构化数据，Agent 容易定位和更新 | Bitable 有 record 数量限制（免费版 5000 条） |
| 天然支持 humans 协作 | 复杂嵌套关系不如 Wiki 灵活 |
| 可按视图分组（决策/人物/项目） |  |

**推荐方案 B**（本地 + 飞书镜像）：
- Agent 侧：直接读写本地 Markdown，无 API 限制，可 Git 版本化
- Human 侧：飞书 Bot 把关键内容同步为只读文档，随时可查看
- 冲突处理：单 Writer + 事件日志（类似 WAL），多 Agent 串行写入

## 实现计划

### 步骤一：目录结构设计

```
~/.closeclaw/team-memory/
├── DECISIONS/          # 技术决策记录
│   └── YYYY-MM-DD--decision-title.md
├── CONTEXT/            # 项目上下文
│   ├── people.md       # 人物关系（Owner / 各 Agent 角色）
│   └── projects.md      # 项目状态
├── LEARNINGS/          # 从错误中学习
│   └── YYYY-MM-DD--feedback.md
└── index.json          # 索引（快速查找用）
```

### 步骤二：Agent 读写接口

- `team_memory_read(query)` — 语义搜索，返回相关片段
- `team_memory_write(type, title, content)` — 追加写入（append-only，决策不修改只新增版本）

### 步骤三：飞书镜像同步

- Bot 定期扫描 `team-memory/` 变化
- 同步到飞书「团队知识库」文件夹（只读文档）
- 人类可直接在飞书编辑，但 Agent 侧不感知（单向同步）

### 步骤四：并发控制

- 多 Agent 写入同一文件时，用文件锁串行化
- 或约定：每类知识只有一个 Agent 负责写入（脑暴虾写 DECISIONS，计划虾写 PROGRESS）

## 扩展性

- 未来可接入 embedding 做 semantic search
- 可接入 Git hook，自动 commit 并通知 Owner
