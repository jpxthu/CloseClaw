# config

## 概述

- 关联需求文档：[requirements/config.md](../../requirements/config.md)
- 配置模块管理 CloseClaw 的运行时配置文件：配置按职责拆分为独立的结构化配置文件，由 ConfigManager 提供统一的读写入口、变更校验、备份保护和自动回退能力；权限配置文件的内容由权限模块自管，本模块只登记其布局并检测变更（见架构）。

## 架构

### 配置目录结构

配置根目录下的 `config/` 中按职责拆分为多个独立配置文件（多数为 JSON，agents.json 为 JSONC）；单文件损坏不波及其他文件的加载流程（损坏文件的回退与整体启动判定见数据流「启动加载」）。配置根目录（`~/.closeclaw`）由 [platform 配置目录](../platform/config-directory.md) 提供，本模块在其下布局与创建下列子目录（其中 `skills/`、`workflows/` 的内容分别由 skills、workflow 模块读写，`workspaces/` 为运行期数据目录，本模块只负责布局）：

```
~/.closeclaw/
├── config/
│   ├── models.json          # LLM 供应商与模型定义
│   ├── channels.json        # 渠道配置与绑定
│   ├── accounts.json        # IM 平台身份绑定（4 方向：机器人↔Agent、IM 用户↔User）
│   ├── gateway.json         # Gateway 服务配置（监听端口、入站队列容量、超时等）
│   ├── plugins.json         # 插件列表与配置（含各 IM 平台的启用状态）
│   ├── session.json         # 会话生命周期配置（idle 超时、purge TTL、扫描间隔、compaction 阈值等，含 per-agent 与按角色回退结构）
│   ├── memory.json          # 记忆模块全局配置（挖掘、做梦、搜索、遗忘等子系统参数）
│   ├── debug_log.json       # 调试日志配置（最低记录级别、保留天数，语义见 debug_log 模块）
│   ├── agents.json          # Agent 注册清单（显式 ID 列表，JSONC）
│   ├── skills.json          # 全局技能配置（extraDirs 外部复用技能目录）
│   ├── permissions.json     # 权限配置（内容结构见权限模块）
│   ├── credentials/         # 凭据子目录（按 LLM 供应商分文件，与业务配置分开存放）
│   │   ├── glm.json
│   │   ├── minimax.json
│   │   └── ...
│   └── .backups/            # 滚动备份目录
├── agents/
│   └── <agent-id>/
│       ├── config.json      # 单个 Agent 的独立配置
│       ├── permissions.json # Agent 权限配置（内容由权限模块解析）
│       ├── skills/          # Agent 层技能目录（由 Skills 模块扫描）
│       ├── workflows/       # Agent 专属 workflow 定义（由 workflow 模块读取）
│       └── .backups/        # 该 Agent 权限配置的滚动备份（由权限模块维护）
├── daemon.pid               # 运行期 PID 文件（由 platform 进程生命周期接口维护，见 platform/process.md）
├── skills/                  # 全局技能目录（由 Skills 模块扫描）
├── workflows/               # 全局 workflow 定义目录（由 workflow 模块读取）
├── logs/                    # 调试日志输出目录（由 debug_log 框架创建与维护，内容语义见 debug_log 模块）
└── workspaces/              # Agent-User 组合专属工作目录（<agent-id>/<user-id>/，运行期数据目录、非配置项，随 Session 创建初始化并由权限模块强制授权）
```

> Agent 层技能目录位于用户级 Agent 目录（`agents/<agent-id>/skills/`）；项目级技能目录 `<project-root>/.closeclaw/skills/` 由 Skills 模块扫描（属技能模块的目录，与 Agent 配置的项目级无关）。
>
> 权限配置文件（`config/permissions.json`、`agents/<agent-id>/permissions.json`）的路径与变更检测由本模块提供；其内容结构、校验规则、热加载与加载容错、备份的创建与恢复均属权限模块职责（见 [权限配置管理](../permission/permission-configuration.md)）。按需求 [config §F3](../../requirements/config.md) 的例外，权限配置文件恢复失败不拒绝启动，由权限模块降级处理（缺失部分按拒绝，无任何可用配置时退化为默认权限并通知 Owner）。`workspaces/` 下的目录随 Session 创建初始化、由权限模块强制授权（见 [session/working-directory](../session/working-directory.md)、[permission §F3](../../requirements/permission.md)），本模块只登记其在配置根目录下的位置。

### 核心组件

- **ConfigManager**：所有配置读写的统一入口。负责加载所有子配置文件到内存、提供读写接口、管理写入流程（校验 → 备份 → 原子写入 → 更新内存或待重启暂存区）、启动时自动回退损坏文件。权限配置文件不在其加载、写入与校验范围内（见上节说明）。
- **BackupManager**：滚动备份管理。维护每个由本模块加载的配置文件最近有限份数（默认 10 份）的历史备份，存于 `config/.backups/` 下（按原文件路径镜像）且权限与原文件一致（凭据备份同具仅 Owner 可读权限）。备份时机：系统写入前，以及运行期重载校验通过、新内容替代当前生效内容之前（覆盖 Owner 直接编辑配置文件的场景）——均将上一份成功加载的配置内容存入备份（写入路径取写入前的磁盘内容，重载路径取内存中的生效值）。备份覆盖 `config/` 下各业务配置文件（含 `config/debug_log.json`）、`credentials/` 下的凭据文件、`agents/<agent-id>/config.json`；权限配置文件与 `.backups/`、`workspaces/` 不在备份范围。启动时回退到最近可用备份。
- **ConfigProvider 体系**：每个子配置文件对应一个 Provider，封装该子配置的数据结构、校验规则与文件路径（`models.json`、`channels.json`、`gateway.json`、`plugins.json` 等以字段校验为主，职责较复杂的几类如下）。CredentialsProvider、SkillsConfigProvider、AgentsConfigProvider 亦属本体系，AgentDirectoryProvider 与 ConfigReloadManager 独立于本体系（均见下）。

  | Provider | 配置源 | 职责 |
  |----------|--------|------|
  | SessionConfigProvider | `config/session.json` | 解析会话生命周期参数（idle 超时、purge TTL、扫描间隔、compaction 阈值等），支持 per-agent 的 idle 超时与 purge 阈值查询（按 agent_id 与角色回退，字段定义与默认值见 [session-lifecycle](../session/session-lifecycle.md)） |
  | AccountsConfigProvider | `config/accounts.json` | 加载账户身份映射（4 方向：机器人↔Agent、IM 用户↔User）、校验发送者标识。生效类别按方向：机器人↔Agent（入站路由、出站发送通道选择）属重启生效类，随网关重建重新装配；IM 用户↔User 属即时生效类——变更校验通过后重建身份映射快照，下一次消息解析即使用新绑定（本模块实现 [IdentityResolver](../common/core-traits.md)，只读约束针对消费方，快照由本模块自行重建；两个方向的生效时机由设计确定，见需求 [config §F7](../../requirements/config.md)） |
  | MemoryConfigProvider | `config/memory.json` | 解析记忆模块全局配置（挖掘、做梦、搜索、遗忘等子系统参数，字段语义与默认值详见 [memory/config](../memory/config.md)） |
  | DebugLogConfigProvider | `config/debug_log.json` | 解析最低记录级别与保留天数，加载与变更后推送给 debug_log（级别语义与默认值见 [debug_log](../debug_log/README.md)） |

  权限配置文件不设 Provider：`config/permissions.json` 与 `agents/<agent-id>/permissions.json` 的内容结构、校验规则、热加载与加载容错均属权限模块职责（见 [权限配置管理](../permission/permission-configuration.md)），本模块只登记路径、检测变更并向权限模块广播通知。
- **CredentialsProvider**：按供应商分文件加载 `credentials/` 目录下的凭据并在启动时固化，运行时按 models 等业务配置中的供应商引用，从已加载的凭据存储中取值注入。凭据在 `credentials/` 子目录下按供应商分文件独立存放、与业务配置分离，models 等业务配置只引用供应商名称、不包含凭据内容；凭据文件创建时设置仅 Owner 可读的文件系统权限；加载时校验并纠正目录与文件权限为仅 Owner 可读（不合规时纠正并记 WARN）。凭据变更属重启生效类（生效类别判定见需求 [config §F7](../../requirements/config.md)，消费侧见 [llm 模块](../llm/README.md)「模型与凭据的生效机制」）；config 作为重载载体对 `credentials/` 目录提供触发入口——凭据文件变更纳入监听并进入待重启暂存区，变更确认与重启触发见 [hot-reload](hot-reload.md)「重启类变更确认与触发」，重启前各消费方按旧凭据运行。加载失败不阻塞 daemon 启动，仅影响需要该供应商的功能。
- **ConfigReloadManager（独立于 ConfigProvider 体系）**：文件变更监控与热重载，监听范围与增量重载机制见 [hot-reload](hot-reload.md)「核心机制」。对非重启生效类变更增量重载、校验通过后更新内存配置并通过事件通道向各订阅的消费模块发送变更通知（校验失败时保留内存旧配置，失败事件经事件通道发布，由 Daemon 订阅后经 Gateway 系统通知接口推送 Owner）；对重启生效类变更（模型与凭据、机器人↔Agent 绑定、渠道配置、插件列表、Gateway 服务参数等）不即时重载，而是进入待重启暂存区、由 daemon 择机重启网关后生效。生效类别判定见需求 [config §F7](../../requirements/config.md)，字段级拆分与通知机制详见 [hot-reload](hot-reload.md)。
- **SkillsConfigProvider**：管理全局技能配置（`config/skills.json`），其中 `extraDirs` 字段定义外部复用技能目录列表，由 Skills 模块的磁盘加载层作为「外部复用」优先级层扫描（优先级低于全局/Agent/项目层，详见 [skills 模块](../skills/README.md)）。
- **AgentsConfigProvider**：管理用户级 Agent 注册清单 `config/agents.json`，文件为含 `agents` 字段的对象，`agents` 是 Agent ID 数组。只列出已显式注册的 ID，不在列表中的 Agent 即使目录存在也不加载。支持 JSONC 格式，注释掉某行即取消注册。
- **AgentDirectoryProvider**：根据注册清单中的 ID，扫描用户级 `agents/` 目录加载每个 Agent 的 `config.json`，并补齐缺失字段默认值。仅加载注册清单中列出的 ID，目录中存在但未被注册的 Agent 配置会被忽略；清单中的 ID 若无 config.json，则记 WARN 并跳过；其 config.json 存在但无法解析时回退备份恢复，备份全部不可用则拒绝启动（与其它配置文件的恢复语义一致，见 [config §F3](../../requirements/config.md)）。

  AgentDirectoryProvider 独立于 ConfigProvider 体系——它不实现 ConfigProvider 接口，由 ConfigManager 直接持有和调用。启动时从 AgentsConfigProvider 获取注册清单，扫描 agents/ 目录完成加载，并校验初始 Agent 标记至多一个（多于一个时取注册清单中最先出现的那个并记录告警，字段语义见 [agent-config](../agent/agent-config.md)）。热重载时在收到 agents.json 变更通知后重新扫描（详见 [hot-reload](hot-reload.md)）。

子功能文档：

- [hot-reload](hot-reload.md) — 配置文件变更监控与增量热重载

## 数据流

### 启动加载

Config 模块启动时依次执行以下步骤：

1. 加载 `config/` 下所有业务配置文件（含 agents.json，由 AgentsConfigProvider 加载注册清单；子目录 `credentials/` 单独加载，见步骤 2；`agents/<agent-id>/config.json` 按注册清单加载，见步骤 4）。各文件独立解析与校验，互不影响。权限配置文件不在本步骤范围内——`config/permissions.json` 与各 Agent 目录下的 `permissions.json` 由权限模块自行加载（见 [权限配置管理](../permission/permission-configuration.md)）。
   - 解析成功且校验通过 → 补齐缺失字段默认值 → 加载到内存。
   - 解析失败 → 由 BackupManager 依次回退较近的备份，直至找到可解析且校验通过的备份（成功 → 记录 WARN，继续）；本次变更的备份全部不可用 → 返回 Err，拒绝启动。回退成功时把损坏文件另存为 `.corrupt.<时间戳>` 供排查，避免每次启动重复告警。
   - 校验失败 → 同上回退流程。
   - 配置中存在不再支持的旧字段 → 静默忽略，不影响加载；已有配置文件始终可被新版本正常加载（兼容性原则见需求 [config §F6](../../requirements/config.md)），缺失字段按默认值补齐、不阻塞启动。
2. 加载 `credentials/` 目录。单个凭据文件解析或校验失败 → 先按 BackupManager 回退到最近可用备份；仍失败 → 使用空凭据，记 WARN（不阻塞启动，见需求 [config §F5](../../requirements/config.md)）。
3. 跨文件引用校验（非阻塞）：accounts 引用的渠道须存在于 channels、models 引用的供应商须存在于 credentials、Agent 配置引用的模型须存在于 models 且父 Agent 须在注册清单中；引用缺失仅记录 WARN、不阻塞启动（对应功能不可用）。
4. AgentDirectoryProvider 读取注册清单，扫描用户级 agents/ 目录完成加载与默认值补齐，生成 [ResolvedAgentConfig](../common/shared-types.md#resolvedagentconfig--subagentsconfig--memoryconfig)[]（流程详见下方「Agent 配置加载」，字段语义见 [agent-config.md](../agent/agent-config.md)）。
5. 加载流程执行完毕（含非阻塞的 WARN 情形）→ 启动 ConfigReloadManager 并进入运行：按 [hot-reload](hot-reload.md)「核心机制」的监听范围注册文件监听（含 `config/permissions.json` 与 `agents/` 下各 Agent 的配置与权限配置）、后台执行热重载。Daemon 随后进入正常消息循环。

### Agent 配置加载

1. 读取用户级注册清单 `config/agents.json`。
2. 仅加载清单中列出的 ID。
3. 对每个注册 ID，加载 `~/.closeclaw/agents/<id>/config.json`：注册 ID 无 config.json → 记 WARN 跳过；文件损坏且按 BackupManager 回退备份后仍无法解析 → 记录错误并拒绝启动（需求 [config §F3](../../requirements/config.md)）；其余情形按加载结果补齐默认值。
4. 补齐所有字段默认值（其中 memory 段由 memory 模块按字段级合并消费，见 [memory/config](../memory/config.md)）。
5. 生成 ResolvedAgentConfig[]，返回给调用方。

字段语义与加载流程详见 [agent-config.md](../agent/agent-config.md) 架构节；ResolvedAgentConfig 类型定义见 [common/shared-types](../common/shared-types.md#resolvedagentconfig--subagentsconfig--memoryconfig)。

### 配置写入

配置写入不阻塞 Owner 的正常使用。调用配置更新接口，传入目标子配置和新内容，依次执行：

1. 校验新配置值。校验失败 → 立即返回错误，不写任何文件。
2. 创建 `.backups/` 目录（如不存在），备份当前配置文件（文件不存在时跳过备份）。备份失败 → 返回错误，不执行写入。
3. 原子写入新配置：写入临时文件 → 强制刷盘临时文件 → 强制刷盘父目录 → 临时文件重命名为目标文件。任一步失败时原有配置文件保持不变（不会产生部分覆盖或损坏，见需求 [config §F2](../../requirements/config.md)）。
4. 更新内存中的配置缓存（重启生效类变更仅写入待重启暂存区、不即时覆盖运行时内存，见 [hot-reload](hot-reload.md)）。写入成功后按变更类别走与热重载相同的通知与待重启暂存流程（不依赖文件监听事件，见 [hot-reload](hot-reload.md)「变更通知」）。

不阻塞的保证：写入不持全局锁、只按目标文件串行化（同一文件的并发写入排队，不同文件的写入互不阻塞），读取方在写盘完成前仍从内存读旧值；因此配置写入不会阻塞 Owner 的消息处理与其它配置操作，写盘耗时只影响本次写入调用的返回。

### 校验规则

下表列出各文件的校验要点；涉及跨文件引用的检查（accounts 引用的渠道、models 引用的供应商）在数据流「启动加载」步骤 3 统一处理。

| 子配置 | 校验要点 |
|--------|---------|
| models | 供应商 ID 非空、模型 ID 非空、base_url 合法 |
| channels | 渠道类型为已知类型、关联对象（平台/默认 Agent 等）非空 |
| gateway | 监听端口在有效范围、超时非负、入站队列容量为正 |
| plugins | 插件名非空、插件可解析；平台启用状态为布尔且平台类型为已知类型 |
| session | idleMinutes、purgeAfterMinutes 非负，sweeperIntervalSeconds 为正；compaction 相关字段按 [common/shared-types](../common/shared-types.md) 的 CompactConfig 定义校验（阈值为合法百分比，告警阈值大于压缩阈值以保证先告警后压缩）；per-agent 条目与 defaults 回退层按同一规则校验 |
| accounts | 分两类记录：① IM 用户↔User——一个 User 一条记录、标识全局唯一，记录下挂多条绑定，绑定键为「渠道 × 接收方机器人应用 × 发送者标识」且全局唯一；同一渠道下同一发送者标识（跨应用）须归并到同一 User（见 [message_flow §F3](../../requirements/message_flow.md)）；② 机器人↔Agent——键为「渠道 × 机器人应用」，不含 User 标识，同一机器人应用唯一绑定一个 Agent（平台内 1:1），同一 Agent 可对应多个平台的机器人 |
| memory | 各子系统字段按 [memory/config](../memory/config.md) 定义的取值与前置条件校验（enabled 为布尔、模型/路径字段合法、数值参数符合各自语义：权重类可为负、数量与时长类非负；条件性免校验的字段按该定义处理） |
| debug_log | 最低记录级别为五个级别之一、保留天数为正整数（级别语义与默认值见 [debug_log](../debug_log/README.md)） |
| credentials | api_key 非空、供应商 ID 非空 |
| agents | 注册清单为有效 JSONC 格式（含 `agents` 数组），ID 非空且不重复 |
| agents/<agent-id>/config.json | 字段取值与默认值见 [agent-config](../agent/agent-config.md)；初始 Agent 标记至多一个（多于一个取注册清单中最先出现的那个并记录告警） |
| skills | extraDirs 为可解析的目录路径列表（路径不存在时由 Skills 模块在扫描时跳过） |

## 模块关系

> 「上游/下游」指数据流与调用关系（含经 common trait 完成的调用），不等于 crate 依赖；crate 依赖以 [STANDARDS.md 依赖方向允许边表](../STANDARDS.md) 为准。

- **上游**：daemon（启动时加载配置）、CLI（启动时加载运行期配置；配置变更命令，含 `config setup` 交互式配置向导——创建初始 Agent 的配置与凭据文件）、platform（提供配置目录，作为各配置子目录布局的基准）
- **下游**：
  - **直接下游（经 ConfigProvider 查询接口消费配置）**：session（SessionConfigProvider——会话配置参数）、skills（SkillsConfigProvider——extraDirs 外部复用技能目录，详见 [skills 模块](../skills/README.md)）、memory（MemoryConfigProvider——记忆模块全局配置，详见 [memory/config](../memory/config.md)）、llm（models.json 的模型定义与 credentials/ 的凭据，启动时固化为模型注册表，详见 [llm 模块](../llm/README.md)「模型与凭据的生效机制」）、Agent（AgentRegistry 消费 ResolvedAgentConfig，重启或配置变更时由 Daemon 触发替换，详见 [agent-registry](../agent/agent-registry.md)）。
  - **文件级路径消费**：Gateway（accounts.json 机器人↔Agent 绑定，加载后内部引用，做入站路由与出站机器人选择，见需求 [config §F1](../../requirements/config.md)）、权限模块（读取 `config/permissions.json` 与 `agents/<agent-id>/permissions.json`，自行解析、校验并维护加载容错与备份恢复，细节见 [权限配置管理](../permission/permission-configuration.md)）。
  - **Owner 提示的转发**：本模块不直接调用其他模块，运行期校验/解析失败只发布失败事件；Daemon 订阅该事件后经 Gateway 系统通知接口向 Owner 推送提示（启动期恢复失败时进程按拒绝启动处理，无出站通道，仅记录错误日志）。
  - **经 common trait 消费（DI 实现方）**：IM Adapter（本模块实现 [IdentityResolver](../common/core-traits.md) 供其入站身份解析使用，绑定数据来自 accounts.json）。
  - **事件通知订阅方**：ConfigReloadManager 以 publish/subscribe 模式向订阅模块推送变更通知；是否订阅、如何重载由各模块自行决定，不构成调用关系。即时生效类与需要感知变更的消费方订阅通知（当前：权限模块订阅权限配置变更、Daemon 订阅 Agent 配置与注册清单变更并触发 AgentRegistry 替换）；边界生效类消费方（session、memory、skills 等）不靠通知推进，在各自边界点从 ConfigManager 读取最新值；日志级别与保留天数由本模块加载后推送给 debug_log（debug_log 属基础层，不反向依赖配置模块），debug_log 自下一条日志、下一次清理起采用新值。
  - **文件级触发**：重启生效类变更确认后触发 daemon 择机重启网关（见 [hot-reload](hot-reload.md)「重启类变更确认与触发」），属事件触发、非 API 调用。
  本模块不主动调用其他业务模块 API，仅经 platform 获取配置目录、读写文件系统并提供查询接口；向 Owner 的提示由 Daemon 经 Gateway 转发（见上）。
- **共享类型 / 核心 trait**：[common/core-traits](../common/core-traits.md)（实现：IdentityResolver）
- **无关**：processor_chain、tools（无调用关系，通过上层模块间接使用配置）、workflow（本模块只布局其定义目录，定义内容与读取由 workflow 模块负责）
