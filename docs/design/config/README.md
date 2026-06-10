# config

## 概述

配置模块管理 CloseClaw 所有运行时配置。配置按职责拆分为独立的结构化配置文件，通过 ConfigManager 提供统一的读写入口、变更校验、备份保护和自动回退能力。

## 架构

### 配置目录结构

配置目录下按职责拆分为多个独立 JSON 文件，一个文件损坏不影响其他文件的加载：

```
~/.closeclaw/
├── config/
│   ├── models.json          # LLM 供应商与模型定义
│   ├── channels.json        # 渠道配置与绑定
│   ├── gateway.json         # Gateway 服务配置
│   ├── plugins.json         # 插件列表与配置
│   ├── session.json         # 会话生命周期配置（idle 超时、purge TTL、compaction 阈值等）
│   ├── system.json          # 系统级配置（定时任务、钩子、消息等）
│   ├── agents.json          # Agent 注册清单（显式 ID 列表，JSONC）
│   ├── credentials/         # 凭据子目录（按供应商分文件，物理隔离）
│   │   ├── feishu.json
│   │   ├── minimax.json
│   │   └── ...
│   └── .backups/            # 滚动备份目录
├── agents/
│   └── <agent-id>/
│       ├── config.json      # 单个 Agent 的独立配置
│       └── permissions.json # Agent 权限配置
└── skills/                  # Skill 文件目录
```

项目级（可选，由用户自行创建）：

```
<repo>/.closeclaw/
└── agents.json              # 项目级 Agent 注册清单（仅包含项目特有 agent 的 ID）
```

### 核心组件

- **ConfigManager**：所有配置读写的统一入口。负责加载所有子配置文件到内存、提供读写接口、管理写入流程（校验 → 备份 → 原子写入 → 更新内存）、启动时自动回退损坏文件。
- **ConfigProvider 体系**：每个子配置文件对应一个 Provider 实现，封装该子配置的数据结构、校验规则和文件路径。session.json 对应 SessionConfigProvider，负责解析 idle 超时、purge TTL 等会话生命周期参数。
- **BackupManager**：滚动备份管理，每次写入前创建备份，在 `.backups/` 下维护每个配置文件最近 N 份历史备份（命名格式 `<文件名>.<时间戳>.json`），支持回退到最近可用备份。ConfigManager 和 ConfigReloadManager 共用 BackupManager 进行回退保护。
- **ConfigReloadManager**：文件变更监控与热重载，监听配置目录变更事件，增量重载变更文件，校验通过后更新内存配置并推送变更通知到已有会话（详见 hot-reload.md）。
- **凭据分离**：credentials 作为 config 子目录，按供应商分文件存储敏感凭据，与业务配置物理隔离。models 等业务配置只引用供应商名称，凭据由 CredentialsProvider 动态注入。凭据加载失败不阻塞 daemon 启动，仅影响需要该供应商的功能。
- **配置迁移**：支持加载单文件配置结构，自动解析并拆分为 config/ 下各子文件。
- **AgentsConfigProvider**：管理 Agent 注册清单（`config/agents.json`），一个显式的 Agent ID 列表。只列出已显式注册的 ID，不在列表中的 Agent 即使目录存在也不加载。支持 JSONC 格式，注释掉某行即取消注册。
- **AgentDirectoryProvider**：根据注册清单中的 ID，扫描 `agents/` 目录加载每个 Agent 的 `config.json` 和 `permissions.json`。支持多级加载（项目级优先于用户级），同 ID 的配置进行字段级覆盖合并。仅加载注册清单中列出的 ID，目录中存在但未被注册的 Agent 配置会被忽略。

  AgentDirectoryProvider 独立于 ConfigProvider 体系，负责扫描多文件和多级目录的 Agent 配置，由 ConfigManager 直接持有和调用。

子功能文档：

- [hot-reload](hot-reload.md) — 配置文件变更监控与增量热重载

## 数据流

### 启动加载

```
Config 模块启动时
  │
  ├─→ 加载 config/ 下所有配置文件
  │     │     │
  │     │     ├─ 解析成功 & 校验通过 → 加载到内存
  │     │     │
  │     │     ├─ 解析失败 → BackupManager 查找最近备份
  │     │     │     ├─ 备份存在 → 回退到备份文件 → 重试加载
  │     │     │     │     ├─ 成功 → 记录 WARN，继续
  │     │     │     │     └─ 仍失败 → 返回 Err，拒绝启动
  │     │     │     └─ 无备份 → 返回 Err，拒绝启动
  │     │     │
  │     │     └─ 校验失败 → 同上回退流程
  │     │
  │     ├─ 加载 credentials/ 目录
  │     │     └─ 加载失败 → 使用空凭据，记录 WARN（不阻塞启动）
  │     │
  │     └─ 全部加载成功 → 启动 ConfigReloadManager（注册文件监听、热重载）
  │
  └─→ Daemon 正常运行，热重载监听器后台运行
```

### Agent 配置加载

```
读取注册清单（config/agents.json + 项目级 agents.json）
  ↓
取 ID 并集，仅加载清单中列出的 ID
  ↓
对每个注册 ID：
  ├─ 优先加载项目级 agents/<id>/config.json（不存在回退用户级）
  └─ agents/<id>/permissions.json 同优先级独立加载
  ↓
字段级覆盖合并（项目 > 用户）
  ↓
补齐所有字段默认值
  ↓
注册到内存配置表
```

完整合并规则和字段语义详见 [agent-config.md](../agent/agent-config.md) 架构节。

### 配置写入

```
调用配置更新接口，传入目标子配置和新内容
  │
  ├─ 1. 校验新配置值
  │     └─ 校验失败 → 立即返回错误，不写任何文件
  │
  ├─ 2. 备份当前配置文件
  │     └─ 备份失败 → 返回错误，不执行写入
  │
  ├─ 3. 原子写入新配置
  │     ├─ 写入临时文件
  │     ├─ 强制刷盘临时文件
  │     ├─ 强制刷盘父目录
  │     └─ 临时文件重命名为目标文件
  │
  └─ 4. 更新内存中的配置缓存
```

### 校验规则

| 子配置 | 校验要点 |
|--------|---------|
| models | 供应商 ID 非空、模型 ID 非空、base_url 合法、api_key 引用有效 |
| channels | 渠道类型为已知类型、绑定目标存在 |
| gateway | 端口在有效范围、超时非负 |
| plugins | 插件名非空、插件可解析 |
| session | idleMinutes 非负、purgeAfterMinutes 非负、sweeperIntervalSeconds 为正 |
| system | 版本号非空、cron 表达式合法 |
| credentials | 供应商 ID 与 models 引用匹配、api_key 非空 |
| agents | ID 列表为有效 JSONC 格式、每个 ID 对应的 config.json 可解析、parentId 引用的 ID 已注册 |

## 模块关系

- **上游**：daemon（启动时加载配置）、CLI（配置变更命令，含 `config setup` 交互式配置向导）、session（读取会话配置）、agent（提供 Agent 配置文件，Config 启动时扫描加载并合并注册）
- **下游**：无（配置模块不调用其他模块，仅读写文件系统和提供查询接口）
- **无关**：processor_chain、tools、skills（无调用关系，这些模块通过上层模块间接使用配置）
