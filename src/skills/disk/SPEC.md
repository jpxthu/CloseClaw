# Disk Skill 系统规格书

> 本文件描述 `src/skills/disk/` 模块的当前实现状态。

## 1. 模块概述

Disk Skill 模块提供基于文件系统的技能发现机制。扫描层级目录结构，从 `SKILL.md` 文件中解析 YAML frontmatter 元数据，按优先级聚合来自不同来源的技能定义。

核心设计：技能以目录为单位组织，每个技能目录含一个 `SKILL.md`。通过 `ScanConfig` 支持多个扫描路径，按来源优先级去重合并。

边界：
- **依赖**：标准库文件 I/O、serde_yaml、tracing（用于警告日志）
- **被谁依赖**：上游 `skills` 模块通过 `pub mod disk` 暴露本模块
- **不涉及**：技能执行（由 `SkillRegistry` / `Skill::execute` 处理）、权限引擎、网络

---

## 2. 公开接口

### 类型

| 类型 | 功能 |
|------|------|
| `DiskSkill` | 磁盘上发现的技能：来源、manifest、SKILL.md 路径、技能目录路径 |
| `DiskSkillRegistry` | 内存注册表：持有 `Vec<DiskSkill>`，提供 `get` / `list` / `contains` / `filter_by_source` / `len` / `is_empty` |
| `ResolvedSkill<'a>` | 技能解析结果枚举：`Disk(&DiskSkill)` 或 `Bundled(Arc<dyn Skill>)` |
| `ParsedSkill` | 解析结果：manifest、是否仅描述字段、原始 frontmatter 文本 |
| `ScanConfig` | 扫描配置：bundled_dir / extra_dirs / global_dir / project_root / agent_id |
| `SkillSource` | 技能来源枚举：`Bundled` / `ExtraDirs` / `Global` / `Agent` / `Project` |
| `SkillContext` | 执行上下文：`Inline`（默认）或 `Agent { agent_id }` |
| `SkillEffort` | 工作量估算：`Trivial` / `Small` / `Medium` / `Large` / `Unknown` |
| `SkillManifest` | 从 frontmatter 解析的技能元数据（name / description / allowed_tools / when_to_use / context / agent / agent_id / effort / paths / user_invocable） |
| `ParseError` | 解析错误：`MissingDelimiter` / `InvalidYaml` / `MissingDescription` |
| `SkillWatcherHandle` | 文件监听 RAII 句柄，drop 时自动停止 watcher |
| `HotReloadError` | 热重载错误：`WatcherCreate` / `WatchPath` / `NoDirectories` |

### 入口函数

| 函数 | 功能 |
|------|------|
| `parse_skill_md(raw)` | 解析 SKILL.md 文件内容，返回 ParsedSkill 或 ParseError |
| `scan_all_skills(config)` | 扫描所有配置的技能目录，返回按优先级去重的 Vec<DiskSkill> |
| `init_disk_skills(config)` | 初始化磁盘技能注册表：调用 `scan_all_skills`，返回 DiskSkillRegistry |
| `resolve_skill(name, disk_registry, skill_registry)` | 查询路由：先查 DiskSkillRegistry（同步），未命中再查 SkillRegistry（async），返回 ResolvedSkill |
| `start_skill_watcher(skill_dirs, on_change)` | 启动技能目录文件监听，300ms debounce 后触发回调，返回 SkillWatcherHandle |

---

## 3. 架构与结构

### 子模块划分

| 文件 | 职责 |
|------|------|
| `types.rs` | 所有类型定义，包含 SkillManifest、DiskSkill、ParsedSkill、ScanConfig、SkillSource、SkillContext、SkillEffort、ParseError |
| `frontmatter.rs` | SKILL.md YAML frontmatter 解析实现 |
| `loader.rs` | 技能目录扫描实现，按 SkillSource 优先级聚合 |
| `registry.rs` | DiskSkillRegistry 内存注册表：持有 `Vec<DiskSkill>`，提供 `get` / `list` / `contains` / `filter_by_source` / `len` / `is_empty` |
| `resolve.rs` | 技能查询路由函数 `resolve_skill`：先查 DiskSkillRegistry（同步），未命中再查 SkillRegistry（async），返回 `ResolvedSkill` |
| `init.rs` | `init_disk_skills` 初始化入口：调用 `scan_all_skills` 并构造 DiskSkillRegistry |
| `hot_reload.rs` | 文件监听热重载，通过 notify 检测变更 → 300ms debounce → 触发回调 |

### 扫描优先级

从低到高扫描，从高到低覆盖：`Project` → `Agent` → `Global` → `ExtraDirs` → `Bundled`。同名称技能高优先级覆盖低优先级并发出 warn 日志。

### 目录结构约定

每个技能对应一个目录，目录名即技能名，目录内必须有 `SKILL.md` 文件。agent 层级的技能位于 `global_dir/agents/<agent_id>/<skill_name>/SKILL.md`。

### 数据流

`scan_all_skills` → `scan_layer`（每个来源）→ 读取 `SKILL.md` → `parse_skill_md` → `DiskSkill` → 按名称去重 → 返回有序列表

---

## 4. 已知限制

- 扫描是同步阻塞 I/O，在大目录或网络存储上可能较慢
- 不支持嵌套技能目录（只扫描一级子目录）
- 不验证 `allowed_tools` 引用的工具是否存在
- `parse_skill_md` 通过字符串查找 `---` 分隔符，而非完整 YAML 前端标记解析
- agent 层级的技能从 `global_dir/agents/<agent_id>` 推导，不直接配置独立路径