# config

## 概述

- 关联需求文档：[requirements/config.md](../requirements/config.md)
- 配置模块管理 CloseClaw 所有运行时配置。配置按职责拆分为独立的结构化配置文件，通过 ConfigManager 提供统一的读写入口、变更校验、备份保护和自动回退能力。

## 架构

### 配置目录结构

配置目录下按职责拆分为多个独立 JSON 文件，一个文件损坏不影响其他文件的加载：

```
~/.closeclaw/
├── config/
│   ├── models.json          # LLM 供应商与模型定义
│   ├── channels.json        # 渠道配置与绑定
│   ├── accounts.json        # 账户注册与 IM 平台身份绑定
│   ├── gateway.json         # Gateway 服务配置
│   ├── plugins.json         # 插件列表与配置
│   ├── session.json         # 会话生命周期与执行持久化配置（idle 超时、purge TTL、compaction 阈值、plan 归档天数、审计日志存储上限等）
│   ├── media.json           # 媒体存储配置（存储目录、保留期等）
│   ├── system.json          # 系统级配置（定时任务、钩子、消息等）
│   ├── agents.json          # Agent 注册清单（显式 ID 列表，JSONC）
│   ├── skills.json          # 全局技能配置（extraDirs 外部复用技能目录）
│   ├── credentials/         # 凭据子目录（按 LLM 供应商分文件，与业务配置分开存放）
│   │   ├── glm.json
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
├── agents.json              # 项目级 Agent 注册清单（仅包含项目特有 agent 的 ID）
└── agents/
    └── <agent-id>/
        ├── config.json      # 单个 Agent 的独立配置
        └── permissions.json # Agent 权限配置
```

### 核心组件

- **ConfigManager**：所有配置读写的统一入口。负责加载所有子配置文件到内存、提供读写接口、管理写入流程（校验 → 备份 → 原子写入 → 更新内存）、启动时自动回退损坏文件。
- **ConfigProvider 体系**：每个子配置文件对应一个 Provider 实现，封装该子配置的数据结构、校验规则和文件路径。session.json 对应 SessionConfigProvider，负责解析 idle 超时、purge TTL、plan 归档天数、审计日志存储上限等会话生命周期与执行持久化参数。accounts.json 对应 AccountsConfigProvider，负责加载账户身份映射（含 IM 用户→用户 ID 与机器人→Agent 绑定两类承载，生效类别见下）、校验发送者标识与平台对应关系。media.json 对应 MediaConfigProvider，负责解析媒体存储目录、保留期等媒体存储参数（详见 [im_adapter media-store](../im_adapter/media-store.md)）。
- **BackupManager**：滚动备份管理，每次写入前自动创建 `.backups/` 目录（如不存在），在 `.backups/` 下维护每个配置文件最近 N 份历史备份（命名格式 `<文件名>.<时间戳>.json`），支持启动时回退到最近可用备份。
- **ConfigReloadManager**：文件变更监控与热重载，监听 `config/` 目录下的业务配置文件及其子目录 `credentials/` 的变更事件，增量重载变更文件，校验通过后更新内存配置并通过事件通道向各消费模块发送变更通知（详见 hot-reload.md）。
- **CredentialsProvider**：按供应商分文件加载 `credentials/` 目录下的凭据，运行时根据 models 等业务配置中的供应商引用，从已加载的凭据存储中取值注入。凭据变更的生效类别由消费方 LLM 模块判定（属重启生效类，见 [llm 模块](../llm/README.md)「模型与凭据的生效机制」）；config 作为重载载体对 `credentials/` 目录提供触发入口——凭据文件变更纳入监听并进入待重启暂存区，变更确认与重启触发见 [hot-reload](hot-reload.md)「重启类变更确认与触发」，重启前各消费方按旧凭据运行。加载失败不阻塞 daemon 启动，仅影响需要该供应商的功能。
- **凭据分离**：凭据在 `config/credentials/` 子目录下按供应商分文件独立存放，与业务配置分离；models 等业务配置只引用供应商名称，不包含凭据内容。凭据文件创建时设置仅 Owner 可读的文件系统权限。
- **AccountsConfigProvider 账户映射生效类别**：账户映射的两类承载归属不同生效类别——**IM 用户→用户 ID** 变更后即时生效（身份与权限查询引用新映射）；**机器人→Agent 绑定**属重启生效类，变更确认后触发网关重启生效（绑定承载与路由见 [gateway 模块](../gateway/README.md) 路由决策，重启触发见 [hot-reload](hot-reload.md)「重启类变更确认与触发」）。
- **SkillsConfigProvider**：管理全局技能配置（`config/skills.json`），其中 `extraDirs` 字段定义外部复用技能目录列表，由 Skills 模块的磁盘加载层作为「外部复用」优先级层扫描（优先级低于全局/Agent/项目层，详见 [skills 模块](../skills/README.md)），用于复用其他工具链或统一外部技能库。
- **AgentsConfigProvider**：管理 Agent 注册清单（`config/agents.json`），一个显式的 Agent ID 列表。只列出已显式注册的 ID，不在列表中的 Agent 即使目录存在也不加载。支持 JSONC 格式，注释掉某行即取消注册。
- **AgentDirectoryProvider**：根据注册清单中的 ID，扫描 `agents/` 目录加载每个 Agent 的 `config.json`。支持多级加载（项目级优先于用户级），同 ID 的配置进行字段级覆盖合并。仅加载注册清单中列出的 ID，目录中存在但未被注册的 Agent 配置会被忽略。

  AgentDirectoryProvider 独立于 ConfigProvider 体系——它不实现 ConfigProvider 接口，由 ConfigManager 直接持有和调用。启动时从 AgentsConfigProvider 获取注册清单，扫描 agents/ 目录完成多级加载和字段合并。热重载时在收到 agents.json 变更通知后重新扫描。

子功能文档：

- [hot-reload](hot-reload.md) — 配置文件变更监控与增量热重载

## 数据流

### 启动加载

Config 模块启动时依次执行以下步骤：

1. 加载 `config/` 下所有配置文件（含 agents.json，由 AgentsConfigProvider 加载注册清单）。
   - 解析成功且校验通过 → 补齐缺失字段默认值 → 加载到内存。
   - 解析失败 → 由 BackupManager 查找最近备份：备份存在则回退到备份文件后重试加载（成功 → 记录 WARN，继续；仍失败 → 返回 Err，拒绝启动）；无备份 → 返回 Err，拒绝启动。
   - 校验失败 → 同上回退流程。
   - 配置中存在不再支持的旧字段 → 静默忽略，不影响加载。
2. 加载 `credentials/` 目录。加载失败 → 使用空凭据，记录 WARN（不阻塞启动）。
3. AgentDirectoryProvider 读取注册清单：扫描 agents/ 目录（用户级 + 项目级）→ 对每个注册 ID 加载 config.json → 字段级覆盖合并（项目 > 用户）→ 补齐默认值，生成 ResolvedAgentConfig[]（字段定义见 agent/agent-config.md）。
4. 全部加载成功 → 启动 ConfigReloadManager（注册文件监听、热重载）。
5. Daemon 正常运行，热重载监听器后台运行。

### Agent 配置加载

1. 读取注册清单（config/agents.json + 项目级 agents.json），取 ID 并集。
2. 仅加载清单中列出的 ID。
3. 对每个注册 ID，从用户级与项目级两层目录扫描并加载 config.json。
4. 对同 ID 的配置进行字段级覆盖合并（项目 > 用户）。
5. 补齐所有字段默认值。
6. 生成 ResolvedAgentConfig，返回给调用方。

完整合并规则和字段语义详见 [agent-config.md](../agent/agent-config.md) 架构节。

### 配置写入

配置写入在毫秒级完成，不阻塞 Owner 的正常使用。调用配置更新接口，传入目标子配置和新内容，依次执行：

1. 校验新配置值。校验失败 → 立即返回错误，不写任何文件。
2. 创建 `.backups/` 目录（如不存在），备份当前配置文件。备份失败 → 返回错误，不执行写入。
3. 原子写入新配置：写入临时文件 → 强制刷盘临时文件 → 强制刷盘父目录 → 临时文件重命名为目标文件。
4. 更新内存中的配置缓存。

### 校验规则

| 子配置 | 校验要点 |
|--------|---------|
| models | 供应商 ID 非空、模型 ID 非空、base_url 合法、api_key 字段（如有）指向有效的 credentials 供应商条目 |
| channels | 渠道类型为已知类型、绑定目标存在 |
| gateway | 端口在有效范围、超时非负 |
| plugins | 插件名非空、插件可解析 |
| session | idleMinutes 非负、purgeAfterMinutes 非负、sweeperIntervalSeconds 为正、planArchiveDays 非负、auditLogLimit 非负 |
| system | 版本号非空、cron 表达式合法 |
| accounts | 账户 ID 非空且唯一、平台名与 channels 中的渠道对应、发送者标识非空；同一平台内「接收方机器人应用 × 发送者标识」组合唯一 |
| media | 存储目录路径合法、保留期为非负整数（0 表示禁用定期清理）、图片内容阈值为非负整数 |
| credentials | 供应商 ID 与 models 引用匹配、api_key 非空 |
| agents | ID 列表为有效 JSONC 格式、每个 ID 对应的 config.json 可解析 |
| skills | extraDirs 为可解析的目录路径列表（路径不存在时由 Skills 模块在扫描时跳过） |

## 模块关系

- **上游**：daemon（启动时加载配置）、CLI（配置变更命令，含 `config setup` 交互式配置向导）、agent（提供 Agent 配置文件，Config 启动时扫描加载并合并为 ResolvedAgentConfig）
- **下游**：无（配置模块不主动调用其他模块 API，仅读写文件系统和提供查询接口）。ConfigReloadManager 通过事件通道以 publish/subscribe 模式向订阅模块推送变更通知——是否订阅、订阅后如何重载，由各模块自行决定，不构成调用关系。**间接消费方**：session（通过 SessionConfigProvider 查询会话配置参数）、IM Adapter（入站身份映射时查询 accounts.json）、权限模块（延迟加载 agent 配置目录下的 permissions.json 文件，热适应由权限模块内部机制实现）、skills（通过 SkillsConfigProvider 读取 extraDirs 外部复用技能目录，详见 [skills 模块](../skills/README.md)）
- **共享类型 / 核心 trait**：[common/core-traits](../common/core-traits.md)（实现：IdentityResolver）
- **无关**：processor_chain、tools（无调用关系，这些模块通过上层模块间接使用配置）
