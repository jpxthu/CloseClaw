# Permission 模块规格说明书

> 本文件描述 `src/permission/` 模块的精确功能说明，即"系统现在是什么"。
> 不是需求文档，不含开发步骤、issue 号、验收标准或工期估算。
> 按 SPEC_CONVENTION.md v3 标准编写。

---

## 一、模块概述

为 Agent 提供操作授权服务（文件、命令、网络、工具调用、跨 Agent 通信、配置写入）。运行在独立 OS 进程中，通过 Unix domain socket IPC 与 Host 进程通信，实现安全隔离。

---

## 二、公开接口

### 构造

| 接口 | 功能 |
|------|------|
| `PermissionEngine::new` | 从 RuleSet 创建引擎实例，同时构建 O(1) 索引 |
| `RuleBuilder::new` | 创建空 Rule 构建器 |
| `RuleSetBuilder::new` | 创建空 RuleSet 构建器 |
| `ActionBuilder::default` | 创建空 Action 构建器 |
| `ActionBuilder::file` / `command` / `network` / `tool_call` / `inter_agent` / `config_write` | 构建对应类型的 Action |
| `ActionBuilder::allowed_args` / `blocked_args` | 为 Command 设置允许/阻断参数 |
| `ActionBuilder::with_hosts` / `with_ports` | 为 Network 设置主机和端口 |
| `ActionBuilder::with_methods` / `with_agents` / `with_files` | 为各类型设置具体范围 |
| `ActionBuilder::build` | 最终化构建 |
| `Sandbox::new` | 创建沙箱实例（使用默认安全策略） |

### 配置

| 接口 | 功能 |
|------|------|
| `RuleBuilder::name` | 设置规则名称 |
| `RuleBuilder::subject` | 设置 Subject（AgentOnly 或 UserAndAgent） |
| `RuleBuilder::subject_agent` | 设置 AgentOnly subject（支持 Exact/Glob） |
| `RuleBuilder::subject_glob` | 设置 Glob 模式的 AgentOnly subject |
| `RuleBuilder::subject_user_and_agent` | 设置 UserAndAgent subject |
| `RuleBuilder::allow` / `deny` | 设置 Effect |
| `RuleBuilder::action` / `actions` | 添加 Action |
| `RuleBuilder::template` | 设置模板引用 |
| `RuleBuilder::priority` | 设置优先级（默认 0，值越大越先评估） |
| `RuleBuilder::build` | 构建 Rule（校验必填字段） |
| `RuleSetBuilder::version` | 设置版本 |
| `RuleSetBuilder::rule` / `rules` | 添加规则 |
| `RuleSetBuilder::defaults` | 设置所有默认 Effect |
| `RuleSetBuilder::default_file` / `default_command` / `default_network` / `default_inter_agent` / `default_config` | 设置各类默认值 |
| `RuleSetBuilder::template_include` | 添加模板包含 |
| `RuleSetBuilder::agent_creator` | 注册 agent 创建者映射 |
| `RuleSetBuilder::build` | 构建 RuleSet（校验必填字段） |
| `Sandbox::with_policy` | 链式设置安全策略 |

### 主操作

| 接口 | 功能 |
|------|------|
| `PermissionEngine::check` | 简化权限检查（接受字符串 action 名） |
| `PermissionEngine::evaluate` | 完整权限评估流程 |
| `PermissionEngine::reload_rules` | 重新加载 RuleSet（重建索引） |
| `PermissionEngine::load_templates` | 加载模板映射 |
| `PermissionEngine::rebuild_indices_with_rules` | 从给定 RuleSet 重建 O(1) 索引 |
| `Sandbox::spawn` | 启动引擎子进程（fork + exec，等待 socket 就绪） |
| `Sandbox::restart` | 重启引擎子进程 |
| `Sandbox::shutdown` | 关闭引擎子进程 |
| `Sandbox::evaluate` | 通过 IPC 请求引擎评估权限 |
| `Sandbox::reload_rules` | 通过 IPC 重新加载规则 |
| `load_templates_from_dir` | 从目录加载所有 `.json` 模板文件并展开继承链 |
| `Template` / `TemplateSubject` / `load_templates_from_dir` | 模板数据结构和加载接口（见 templates.rs） |

### 查询

| 接口 | 功能 |
|------|------|
| `Sandbox::state` | 查询沙箱进程状态 |
| `glob_match` | Glob 模式匹配（支持 `*`、`**`、`?`） |
| `action_matches_request` | 判断 Action 是否覆盖 PermissionRequestBody 请求 |
| `validate_rule` | 验证单条规则，返回所有错误列表 |
| `validate_ruleset` | 验证整个 RuleSet，返回所有错误列表 |
| `has_deny_rules` | RuleSet 中是否存在 Deny 规则 |
| `has_allow_rules` | RuleSet 中是否存在 Allow 规则 |
| `Rule::validate` | 验证规则自身 |
| `Rule::args_match` | 判断规则 CommandArgs 与请求参数是否匹配 |

### 清理

无。

---

## 三、架构与结构

### 子模块划分

| 子模块 | 职责 |
|--------|------|
| `engine/` | 核心评估逻辑：类型定义、O(1) 索引、评估算法 |
| `actions/` | Action 类型及其 Builder |
| `rules/` | Rule/RuleSet Builder 及验证逻辑 |
| `rules/builder.rs` | RuleBuilder + RuleBuilderError |
| `rules/ruleset_builder.rs` | RuleSetBuilder + RuleSetBuilderError |
| `rules/validation.rs` | 规则和规则集验证辅助函数 |
| `templates.rs` | 模板加载与继承展开 |
| `sandbox/` | OS 进程隔离、IPC 通信、安全策略 |
| `sandbox/mod.rs` | Sandbox 生命周期管理、SandboxState、SandboxError |
| `sandbox/ipc.rs` | IpcChannel、SandboxRequest/SandboxResponse IPC 消息 |
| `sandbox/security.rs` | SecurityPolicy、seccomp/landlock 平台策略 |

### 数据流（Host → Engine 子进程）

```
Host 进程                              Engine 子进程
   │                                        │
   │  Sandbox::spawn()                      │
   │  → fork + exec 当前二进制              │
   │  → Unix socket 连接                    │
   │                                        │
   │  Sandbox::evaluate(request)             │
   │  → IpcChannel::call()                  │
   │  → SandboxRequest::Evaluate ──────────►│  IpcChannel::serve() 接收
   │                                      │  → PermissionEngine::evaluate()
   │◄───────────────────────────────────│  → SandboxResponse::PermissionResponse
   │  Result<PermissionResponse>          │
```

### 评估算法（5 步）

1. **Creator 规则短路**：若 caller.user_id == agent_creators[agent_id]，直接 Allow
2. **构建候选规则列表**：O(1) 索引查找 → Glob 回退
3. **按 priority 降序排序**
4. **模板展开**：对 template 引用替换为模板中的实际 actions
5. **AWS IAM 风格求值**（Deny 优先）：遍历匹配规则，遇 Deny 立即返回；无 Deny 但有匹配规则 → Allow；无匹配 → 默认策略

### O(1) 索引

引擎内部维护两张索引：
- `agent_rule_index: HashMap<String, Vec<usize>>` — agent_id → 规则下标
- `user_agent_rule_index: HashMap<String, Vec<usize>>` — `"user_id:agent_id"` → 规则下标

查找时先查索引，索引无结果才做 Glob 遍历。

### 类型继承关系

```
PermissionRequest
├── WithCaller { caller: Caller, request: PermissionRequestBody }
└── Bare(PermissionRequestBody)

PermissionRequestBody
├── FileOp { path: String, operation: String }
├── CommandExec { command: String, args: Vec<String> }
├── NetOp { operation: String, host: String, port: u16 }
├── ToolCall { skill: String, method: String }
├── InterAgentMsg { agent: String }
└── ConfigWrite { path: String }

Action
├── File { operation: String, paths: Vec<String> }
├── Command { command: String, args: CommandArgs }
├── Network { hosts: Vec<String>, ports: Vec<u16> }
├── ToolCall { skill: String, methods: Vec<String> }
├── InterAgent { agents: Vec<String> }
├── ConfigWrite { files: Vec<String> }
└── All

Rule
├── name: String
├── subject: Subject
├── effect: Effect
├── actions: Option<Vec<Action>>  （与 template 二选一）
├── template: Option<TemplateRef> （与 actions 二选一）
└── priority: i32

Subject
├── AgentOnly { agent: String, match_type: MatchType }
└── UserAndAgent { user_id: String, agent: String, match_type: MatchType }
```

### 模块导出（mod.rs 顶层）

```rust
pub use engine::{ glob_match, Action, Caller, CommandArgs,
    Defaults, Effect, MatchType, PermissionEngine, PermissionRequest,
    PermissionRequestBody, PermissionResponse, Rule, RuleSet, Subject, TemplateRef };
pub use rules::{ validation, RuleBuilder, RuleSetBuilder,
    RuleBuilderError, RuleSetBuilderError };
```

`glob_match` 在 `engine` 子模块内可直接访问，通过根模块也可访问。`action_matches_request` 仅在 `engine` 子模块内导出，**未**在根模块 re-export。

`Sandbox`/`SandboxState`/`SandboxError`/`SecurityPolicy`/`IpcChannel`/`SandboxRequest`/`SandboxResponse`/sandbox 常量 **未**在顶层 re-export，通过 `closeclaw::permission::sandbox::Xxx` 访问。

---

## 四、已知偏差（代码 vs 文档）

| 偏差 | 类型 | 说明 |
|------|------|------|
| `PermissionRequest::WithCaller` 字段名 | 错误 | 文档描述为 `body`，代码实际为 `request` |
| `PermissionEngine::load_templates` 无 API 文档 | 少了 | 公开方法，但接口文档缺失 |
| `PermissionEngine::rebuild_indices_with_rules` 无文档 | 少了 | 内部辅助方法公开可见，无文档 |
| 模板系统类型无 API 文档 | 少了 | `Template`/`TemplateSubject`/`load_templates_from_dir` 等无文档 |
| `ActionBuilder` 链式方法无文档 | 少了 | 全部公开方法无 API 文档 |
| `RuleBuilder`/`RuleSetBuilder` 链式方法部分无文档 | 少了 | 部分方法无文档 |
| `SandboxRequest`/`SandboxResponse` 无文档 | 少了 | IPC 消息类型零文档 |
| `SecurityPolicy::apply()` 为 stub | 说明 | seccomp 和 landlock 均未实际激活内核级限制，文档如实说明 |

---

*最后更新：2026-04-14（v3 重写）*
