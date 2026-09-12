# config

## 概述

- 关联需求文档：[requirements/config.md](../../requirements/config.md)
- 配置模块管理 CloseClaw 所有运行时配置。配置按职责拆分为独立的结构化配置文件，通过 ConfigManager 提供统一的读写入口、变更校验、备份保护和自动回退能力。

## 架构

### 配置目录结构

配置目录下按职责拆分为多个独立配置文件（多数为 JSON，agents.json 为 JSONC）；单文件损坏不波及其他文件的加载流程（损坏文件的回退与整体启动判定见数据流「启动加载」）。配置根目录（`~/.closeclaw`）由 [platform 配置目录](../platform/config-directory.md) 提供，本模块在其下布局各子目录：

```
~/.closeclaw/
├── config/
│   ├── models.json          # LLM 供应商与模型定义
│   ├── channels.json        # 渠道配置与绑定
│   ├── accounts.json        # 账户注册与 IM 平台身份绑定
│   ├── gateway.json         # Gateway 服务配置（监听端口、入站队列容量、超时等）
│   ├── plugins.json         # 插件列表与配置
│   ├── session.json         # 会话生命周期与执行持久化配置（idle 超时、purge TTL、扫描间隔、compaction 阈值、plan 归档天数、审计日志存储上限等）
│   ├── memory.json          # 记忆模块全局配置（挖掘、做梦、搜索、遗忘等子系统参数）
│   ├── media.json           # 媒体存储配置（存储目录、保留期、图片内容阈值等）
│   ├── agents.json          # Agent 注册清单（显式 ID 列表，JSONC）
│   ├── skills.json          # 全局技能配置（extraDirs 外部复用技能目录）
│   ├── credentials/         # 凭据子目录（按 LLM 供应商分文件，与业务配置分开存放）
│   │   ├── glm.json
│   │   ├── minimax.json
│   │   └── ...
│   └── .backups/            # 滚动备份目录（不含凭据与 Agent 配置）
├── agents/
│   └── <agent-id>/
│       ├── config.json      # 单个 Agent 的独立配置
│       ├── permissions.json # Agent 权限配置
│       └── skills/          # Agent 层技能目录（由 Skills 模块扫描）
└── skills/                  # 全局技能目录（由 Skills 模块扫描）
```

项目级（可选，由用户自行创建）：

```
<project-root>/.closeclaw/
├── agents.json              # 项目级 Agent 注册清单（仅包含项目特有 agent 的 ID）
├── agents/
│   └── <agent-id>/
│       ├── config.json      # 单个 Agent 的独立配置
│       └── permissions.json # Agent 权限配置
└── skills/                  # 项目级技能目录（由 Skills 模块扫描）
```

> Agent 层技能目录仅存在于用户级 Agent 目录（`agents/<agent-id>/skills/`）；项目级 Agent 目录不含该子目录。

### 核心组件

- **ConfigManager**：所有配置读写的统一入口。负责加载所有子配置文件到内存、提供读写接口、管理写入流程（校验 → 备份 → 原子写入 → 更新内存）、启动时自动回退损坏文件。
- **BackupManager**：滚动备份管理。每次写入前自动创建 `.backups/` 目录（如不存在），在 `.backups/` 下维护 `config/` 下每个业务配置文件最近有限份数的历史备份（在原名后追加 `.<时间戳>.json` 后缀），支持启动时回退到最近可用备份。备份覆盖 `config/` 下的业务配置文件（不含 `credentials/` 凭据文件与 `agents/` 下的 Agent 配置、权限文件）。
- **ConfigProvider 体系**：每个子配置文件对应一个 Provider，封装该子配置的数据结构、校验规则与文件路径（`models.json`、`channels.json`、`gateway.json`、`plugins.json` 等以字段校验为主，职责较复杂的几类如下）。CredentialsProvider、SkillsConfigProvider、AgentsConfigProvider 亦属本体系，AgentDirectoryProvider 与 ConfigReloadManager 独立于本体系（均见下）。

  | Provider | 配置源 | 职责 |
  |----------|--------|------|
  | SessionConfigProvider | `config/session.json` | 解析会话生命周期与执行持久化参数（idle 超时、purge TTL、扫描间隔、compaction 阈值、plan 归档天数、审计日志存储上限等），支持 per-agent 的 idle 超时与 purge 阈值查询（按 agent_id 与角色，字段定义见 [session-lifecycle](../session/session-lifecycle.md)） |
  | AccountsConfigProvider | `config/accounts.json` | 加载账户身份映射（IM 用户→用户 ID、机器人→Agent 绑定两类）、校验发送者标识。两类映射生效类别不同：IM 用户→用户 ID 即时生效，机器人→Agent 绑定属重启生效类（绑定承载与路由见 [gateway 模块](../gateway/README.md) 路由决策） |
  | MediaConfigProvider | `config/media.json` | 解析媒体存储目录、保留期、图片内容阈值等媒体存储参数（详见 [im_adapter media-store](../im_adapter/media-store.md)） |
  | MemoryConfigProvider | `config/memory.json` | 解析记忆模块全局配置（挖掘、做梦、搜索、遗忘等子系统参数，字段语义与默认值详见 [memory/config](../memory/config.md)） |
- **CredentialsProvider**：按供应商分文件加载 `credentials/` 目录下的凭据并在启动时固化，运行时按 models 等业务配置中的供应商引用，从已加载的凭据存储中取值注入。凭据在 `credentials/` 子目录下按供应商分文件独立存放、与业务配置分离，models 等业务配置只引用供应商名称、不包含凭据内容；凭据文件创建时设置仅 Owner 可读的文件系统权限。凭据变更属重启生效类（生效类别判定见需求 [config §F7](../../requirements/config.md)，消费侧见 [llm 模块](../llm/README.md)「模型与凭据的生效机制」）；config 作为重载载体对 `credentials/` 目录提供触发入口——凭据文件变更纳入监听并进入待重启暂存区，变更确认与重启触发见 [hot-reload](hot-reload.md)「重启类变更确认与触发」，重启前各消费方按旧凭据运行。加载失败不阻塞 daemon 启动，仅影响需要该供应商的功能。
- **ConfigReloadManager（独立于 ConfigProvider 体系）**：文件变更监控与热重载，监听 `config/` 目录下的业务配置文件及其子目录 `credentials/` 的变更事件。对非重启生效类变更增量重载、校验通过后更新内存配置并通过事件通道向各订阅的消费模块发送变更通知（校验失败时保留内存旧配置并经 IM 通知 Owner，详见 [hot-reload](hot-reload.md)）；对重启生效类变更（模型与凭据、机器人绑定、Gateway 服务参数等）不即时重载，而是进入待重启暂存区、由 daemon 择机重启网关后生效。同一文件内若含不同生效类别的字段（如 accounts.json），按字段各自类别分别处理。生效类别判定见需求 [config §F7](../../requirements/config.md)，机制详见 [hot-reload](hot-reload.md)。
- **SkillsConfigProvider**：管理全局技能配置（`config/skills.json`），其中 `extraDirs` 字段定义外部复用技能目录列表，由 Skills 模块的磁盘加载层作为「外部复用」优先级层扫描（优先级低于全局/Agent/项目层，详见 [skills 模块](../skills/README.md)）。
- **AgentsConfigProvider**：管理 Agent 注册清单（用户级 `config/agents.json` + 项目级 `<project-root>/.closeclaw/agents.json`，取 ID 并集），一个显式的 Agent ID 列表。只列出已显式注册的 ID，不在列表中的 Agent 即使目录存在也不加载。支持 JSONC 格式，注释掉某行即取消注册。
- **AgentDirectoryProvider**：根据注册清单中的 ID，扫描 `agents/` 目录加载每个 Agent 的 `config.json`。支持多级加载（项目级优先于用户级），同 ID 的配置进行字段级覆盖合并。仅加载注册清单中列出的 ID，目录中存在但未被注册的 Agent 配置会被忽略；注册清单中的 ID 若在用户级与项目级两层均无 config.json、或其 config.json 无法解析，则记 WARN 并跳过。

  AgentDirectoryProvider 独立于 ConfigProvider 体系——它不实现 ConfigProvider 接口，由 ConfigManager 直接持有和调用。启动时从 AgentsConfigProvider 获取注册清单，扫描 agents/ 目录完成多级加载和字段合并。热重载时在收到 agents.json 变更通知后重新扫描（详见 [hot-reload](hot-reload.md)）。

子功能文档：

- [hot-reload](hot-reload.md) — 配置文件变更监控与增量热重载

## 数据流

### 启动加载

Config 模块启动时依次执行以下步骤：

1. 加载 `config/` 下所有业务配置文件（含 agents.json，由 AgentsConfigProvider 加载注册清单；子目录 `credentials/` 单独加载，见步骤 2）。各文件独立解析与校验，互不影响。
   - 解析成功且校验通过 → 补齐缺失字段默认值 → 加载到内存。
   - 解析失败 → 由 BackupManager 查找最近备份：备份存在则回退到备份文件后重试加载（成功 → 记录 WARN，继续；仍失败 → 返回 Err，拒绝启动）；无备份 → 返回 Err，拒绝启动。
   - 校验失败 → 同上回退流程。
   - 配置中存在不再支持的旧字段 → 静默忽略，不影响加载。
2. 加载 `credentials/` 目录。加载失败 → 使用空凭据，记录 WARN（不阻塞启动）。
3. 跨文件引用校验（非阻塞）：accounts 引用的渠道须存在于 channels、models 引用的供应商须存在于 credentials；引用缺失仅记录 WARN、不阻塞启动（对应功能不可用）。
4. AgentDirectoryProvider 读取注册清单，扫描 agents/ 目录（用户级 + 项目级）完成多级加载与字段级合并，生成 ResolvedAgentConfig[]（流程详见下方「Agent 配置加载」，字段定义见 [agent-config.md](../agent/agent-config.md)）。
5. 加载流程执行完毕（含非阻塞的 WARN 情形）→ 启动 ConfigReloadManager 并进入运行：注册文件监听、后台执行热重载。Daemon 随后进入正常消息循环。

### Agent 配置加载

1. 读取注册清单（config/agents.json + 项目级 agents.json），取 ID 并集。
2. 仅加载清单中列出的 ID。
3. 对每个注册 ID，从用户级与项目级两层目录扫描并加载 config.json；若两层均无该 ID 的 config.json、或其 config.json 无法解析，记 WARN 并跳过该 ID（不生成 ResolvedAgentConfig）。
4. 对同 ID 的配置进行字段级覆盖合并（项目 > 用户）。
5. 补齐所有字段默认值。
6. 生成 ResolvedAgentConfig[]，返回给调用方。

完整合并规则和字段语义详见 [agent-config.md](../agent/agent-config.md) 架构节。

### 配置写入

配置写入不阻塞 Owner 的正常使用。调用配置更新接口，传入目标子配置和新内容，依次执行：

1. 校验新配置值。校验失败 → 立即返回错误，不写任何文件。
2. 创建 `.backups/` 目录（如不存在），备份当前配置文件（文件不存在时跳过备份）。备份失败 → 返回错误，不执行写入。
3. 原子写入新配置：写入临时文件 → 强制刷盘临时文件 → 强制刷盘父目录 → 临时文件重命名为目标文件。
4. 更新内存中的配置缓存（重启生效类变更仅写入待重启暂存区、不即时覆盖运行时内存，见 [hot-reload](hot-reload.md)）。

### 校验规则

下表列出各文件的校验要点；涉及跨文件引用的检查（accounts 引用的渠道、models 引用的供应商）在数据流「启动加载」步骤 3 统一处理。

| 子配置 | 校验要点 |
|--------|---------|
| models | 供应商 ID 非空、模型 ID 非空、base_url 合法 |
| channels | 渠道类型为已知类型、绑定目标非空 |
| gateway | 监听端口在有效范围、超时非负、入站队列容量为正 |
| plugins | 插件名非空、插件可解析 |
| session | idleMinutes 非负、purgeAfterMinutes 非负、sweeperIntervalSeconds 为正、planArchiveDays 非负、auditLogLimit 非负；compaction 告警阈值与压缩阈值为合法百分比（按上下文窗口剩余空间触发，告警阈值大于压缩阈值以保证先告警后压缩） |
| accounts | 账户 ID 非空且唯一、发送者标识非空；同一渠道内「接收方机器人应用 × 发送者标识」组合唯一；同一机器人应用唯一绑定一个 Agent |
| media | 存储目录路径合法、保留期为非负整数（0 表示禁用定期清理）、图片内容阈值为非负整数 |
| memory | 各子系统 enabled 为布尔、模型/路径等字段合法、数值参数为非负（字段定义见 [memory/config](../memory/config.md)） |
| credentials | api_key 非空、供应商 ID 非空 |
| agents | ID 列表为有效 JSONC 格式 |
| skills | extraDirs 为可解析的目录路径列表（路径不存在时由 Skills 模块在扫描时跳过） |

## 模块关系

- **上游**：daemon（启动时加载配置）、CLI（配置变更命令，含 `config setup` 交互式配置向导）、Agent（提供 Agent 配置文件，Config 启动时扫描加载并合并为 ResolvedAgentConfig）、platform（提供配置目录，作为各配置子目录布局的基准）
- **下游**：无直接调用（配置模块不主动调用其他业务模块 API，仅经 platform 获取配置目录并读写文件系统、提供查询接口）。ConfigReloadManager 通过事件通道以 publish/subscribe 模式向订阅模块推送变更通知——是否订阅、订阅后如何重载，由各模块自行决定，不构成调用关系；重启生效类变更确认后触发 daemon 择机重启网关（见 [hot-reload](hot-reload.md)「重启类变更确认与触发」，属事件触发、非 API 调用）。**间接消费方**：session（通过 SessionConfigProvider 查询会话配置参数）、Gateway（读取机器人→Agent 绑定）、IM Adapter（入站身份映射时查询 accounts.json）、权限模块（延迟加载 Agent 配置目录下的 permissions.json 文件，热适应由权限模块内部机制实现，不经 config 变更通知）、skills（通过 SkillsConfigProvider 读取 extraDirs 外部复用技能目录，详见 [skills 模块](../skills/README.md)）、memory（通过 MemoryConfigProvider 读取记忆模块全局配置，详见 [memory/config](../memory/config.md)）
- **共享类型 / 核心 trait**：[common/core-traits](../common/core-traits.md)（实现：IdentityResolver）
- **无关**：processor_chain、tools（无调用关系，这些模块通过上层模块间接使用配置）
