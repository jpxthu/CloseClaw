# Plan: Agent Config - model 参数 + 项目级 agents.json

> ⚠️ **状态管理规范**：所有情况下都应尽可能避免全局变量，详见 `skills/avoid-global-state/SKILL.md`。

## 来源
design-doc: docs/design/agent/agent-config.md

## 项目路径
/home/admin/code/closeclaw

## 目的
修复 design doc `agent-config.md` 中发现的 2 个 gap：
1. `sessions_spawn` 工具 schema 声明了 `model` 参数但从未解析和应用（设计文档要求的模型优先级链第一层是死代码）
2. 项目级 `agents.json` 注册清单未加载（设计文档要求加载用户级 + 项目级两份清单并取 ID 并集）

## 思路

### Gap 1 - model 参数
设计文档定义的模型解析优先级：`显式 model 参数 > 父 agent.subagents.model > 目标 agent.model > 系统默认`

当前代码（`src/gateway/session_manager/spawn.rs:152`）只实现了 `目标 agent.model > 系统默认`。需要：
- 在 `SpawnArgs` 中新增 `model` 字段，在 `parse_args()` 中解析
- 在 `create_child()` 和 `create_child_session()` 中传递 `model` 参数
- 在 `create_child_session` 中实现完整优先级链：显式 model 参数 > 父 agent 的 `subagents.model` > 目标 agent 的 `model` > 系统默认

### Gap 2 - 项目级 agents.json
当前 `agent_loader.rs:34` 只从 `self.config_dir.join("agents.json")`（用户级）加载注册清单。需要：
- 额外从 `<repo>/.closeclaw/agents.json`（项目级）加载注册清单
- 两份清单取 ID 并集（同 ID 项目覆盖用户，注释掉的 ID 跳过）
- 复用已有的 `AgentDirectoryProvider` 做后续的 per-agent config 加载（已支持两级目录）

开发必读：
- **必须**阅读 CONTRIBUTING.md `CONTRIBUTING.md`
- **必须**阅读项目 design doc `docs/design/agent/agent-config.md`

### Step 1.1：SpawnArgs 新增 model 字段
修改 `src/tools/builtin/sessions_spawn.rs`：
- `SpawnArgs` 结构体新增 `model: Option<String>` 字段
- `parse_args()` 中解析 `"model"` 参数：`args.get("model").and_then(|v| v.as_str()).map(String::from)`
- `Ok(SpawnArgs { ... })` 中包含 `model` 字段

验收：`cargo check` 通过，SpawnArgs 包含 model 字段，parse_args 正确解析。

### Step 1.2：传递 model 参数到 create_child_session
修改 `src/tools/builtin/sessions_spawn.rs`：
- `create_child()` 方法签名新增 `model: Option<&str>` 参数
- `call()` 中将 `spawn_args.model.as_deref()` 传递给 `create_child()`

修改 `src/gateway/session_manager/spawn.rs`：
- `create_child_session()` 方法签名新增 `model: Option<&str>` 参数
- 在模型解析处实现完整优先级链：
  ```rust
  let model = model_override  // 显式 model 参数
      .or(config.subagents.model.clone())  // 父 agent.subagents.model
      .or(config.model.clone())  // 目标 agent.model
      .unwrap_or_else(|| "default".to_string());  // 系统默认
  ```

验收：`cargo check` 通过，model 参数从 tool call 传递到 session 创建。

### Step 1.3：项目级 agents.json 注册清单加载
修改 `src/config/agent_loader.rs`：
- `load_agents()` 方法签名新增 `repo_root: Option<&Path>` 参数（如已有则确认传递）
- 在加载用户级 `agents.json` 后，额外加载 `<repo>/.closeclaw/agents.json`（如存在）
- 解析两份清单，取 ID 并集（`HashSet` 合并）
- 将合并后的 ID 列表传递给 `AgentDirectoryProvider`
- 注意：`reload_agents()` 也需要传递 `repo_root`（可能需要从 ConfigManager 获取）

验收：`cargo check` 通过，两份 agents.json 都被加载，ID 取并集。

### Step 1.4：UT 编写
修改/新增测试文件：
- `sessions_spawn` model 参数解析测试：验证 model 字段正确解析（有值/无值/缺失）
- `create_child_session` 模型优先级测试：验证 4 层优先级链
- `agent_loader` 项目级 agents.json 测试：验证 ID 并集合并逻辑

验收：`cargo test` 通过，覆盖所有新增代码逻辑分支。

### Step 1.5：修复模型优先级链 — 父 agent.subagents.model 未正确使用（E2 review 发现）

**问题**：Step 1.2 实现的优先级链中 `config.subagents.model` 使用的是**目标 agent** 的配置，但设计文档要求是**父 agent** 的 `subagents.model`。

修改 `src/tools/builtin/sessions_spawn.rs`：
- `call()` 中，在获取 `parent_session_id` 和 `config`（目标 agent）之后，查找父 agent 的配置：
  ```rust
  let parent_agent_id = self.session_manager.get_chat_id(parent_session_id).await;
  let parent_subagents_model = parent_agent_id
      .as_ref()
      .and_then(|id| self.config_manager.agent(id))
      .and_then(|c| c.subagents.model.clone());
  ```
- `create_child()` 方法签名新增 `parent_subagents_model: Option<&str>` 参数
- 将 `parent_subagents_model.as_deref()` 传递给 `create_child()`

修改 `src/gateway/session_manager/spawn.rs`：
- `create_child_session()` 方法签名新增 `parent_subagents_model: Option<&str>` 参数
- 修正模型优先级链：
  ```rust
  let model = model_override                          // 1. 显式 model 参数
      .map(String::from)
      .or(parent_subagents_model.map(String::from))   // 2. 父 agent.subagents.model
      .or(config.model.clone())                        // 3. 目标 agent.model
      .unwrap_or_else(|| "default".to_string());       // 4. 系统默认
  ```
- 删除 `config.subagents.model` 相关代码（该字段应由父 agent 侧消费，非目标 agent 侧）

修改测试文件：适配新签名，更新测试用例验证正确的优先级链。

验收：`cargo check` 通过，优先级链与设计文档一致。

### Step 1.6：修复 repo_root 未赋值 — reload_agents() 热重载丢失项目级配置（E2 review 发现）

**问题**：`ConfigManager.repo_root` 初始化为 `None` 且无处赋值，`reload_agents()` 读取 `self.repo_root` 始终为 `None`。

修改 `src/config/agent_loader.rs`：
- 在 `load_agents()` 方法开头，将 `repo_root` 参数保存到 `self.repo_root`：
  ```rust
  // Persist repo_root for reload_agents()
  if repo_root.is_some() {
      *self.repo_root_slot() = repo_root.map(Path::to_path_buf);
  }
  ```
  或者直接在 `load_agents()` 开头赋值：
  ```rust
  self.repo_root = repo_root.map(|p| p.to_path_buf());
  ```

验收：`cargo check` 通过，首次 `load_agents(repo_root)` 后 `self.repo_root` 被正确赋值，`reload_agents()` 能加载项目级 agents.json。

## 变更文件清单

| 文件 | 变更类型 | 说明 |
|------|---------|------|
| `src/tools/builtin/sessions_spawn.rs` | 修改 | SpawnArgs 新增 model，传递到 create_child |
| `src/gateway/session_manager/spawn.rs` | 修改 | create_child_session 新增 model 参数，实现优先级链 |
| `src/config/agent_loader.rs` | 修改 | 加载项目级 agents.json，ID 并集 |
| 测试文件 | 新增/修改 | 覆盖新增逻辑 |

## 约束
- 文件行数 ≤ 500（含测试）
- 单行宽度 ≤ 100 字符
- 函数体 ≤ 50 行
- 函数参数 ≤ 6
- 模块嵌套 ≤ 3 层
