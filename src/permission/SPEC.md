# Permission 模块规格说明书

> 本文件描述 `src/permission/` 模块的精确功能说明，即"系统现在是什么"。
> 不是需求文档，不含开发步骤、issue 号、验收标准或工期估算。
> 按 SPEC_CONVENTION.md v3 标准编写。

---

## 一、模块概述

为 Agent 提供操作授权服务（文件、命令、网络、工具调用、跨 Agent 通信、配置写入）。运行在独立 OS 进程中，通过 Unix domain socket IPC 与 Host 进程通信，实现安全隔离。

`approval` 子模块提供内存审批队列，用于管理被 deny 且需要 owner 审批的操作。队列基于 `PermissionRequestBody` 的 SHA256 哈希做去重，审批决策后触发回调恢复 agent session。

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
| `RuleSetBuilder::default_file` / `default_command` / `default_network` / `default_inter_agent` / `default_config` / `default_tool_call` | 设置各类默认值 |
| `RuleSetBuilder::template_include` | 添加模板包含 |
| `RuleSetBuilder::agent_creator` | 注册 agent 创建者映射 |
| `RuleSetBuilder::build` | 构建 RuleSet（校验必填字段） |
| `Sandbox::with_policy` | 链式设置安全策略 |

### 主操作

| 接口 | 功能 |
|------|------|
| `PermissionEngine::check` | 简化权限检查（接受字符串 action 名，内部调用 `evaluate` 并传入 `None` 作为 `extra_deny_subjects`） |
| `PermissionEngine::evaluate` | 完整权限评估流程，支持 `extra_deny_subjects` 参数在标准评估后对额外 subject 做 deny 覆盖扫描 |
| `PermissionEngine::get_agent_deny_subjects` | 提取父 agent 的 AgentOnly + Deny 规则 subject（将 agent 字段替换为 child_id），用于子 agent 权限递减链 |
| `PermissionEngine::reload_rules` | 重新加载 RuleSet（重建索引） |
| `PermissionEngine::load_templates` | 加载模板映射 |
| `PermissionEngine::rebuild_indices_with_rules` | 从给定 RuleSet 重建 O(1) 索引 |
| `ApprovalQueue::new` | 创建空审批队列 |
| `ApprovalQueue::enqueue` | 添加待审批请求，基于 body SHA256 去重，callback 在审批决策时触发 |
| `ApprovalQueue::approve` | 批准请求，触发 Approve 回调 |
| `ApprovalQueue::deny` | 拒绝请求，触发 Deny 回调 |
| `ApprovalQueue::clear` | 清空队列，所有 pending 触发 Deny 回调 |
| `ApprovalQueue::get_pending` | 查询 pending 条目 |
| `ApprovalQueue::compute_operation_key` | 计算请求体的 SHA256 去重 key |
| `approval::RequestId` | 待审批请求唯一标识（String 别名） |
| `approval::OperationKey` | 请求体 SHA256 十六进制字符串（String 别名） |
| `approval::ApproveOrDeny` | 审批决策枚举（Approve / Deny） |
| `approval::RejectReason` | 入队拒绝原因枚举（Duplicate） |
| `approval::PendingApproval` | 待审批条目结构体（request_id, caller, operation_key, operation_desc, risk_level, rule_version, session_resume, created_at） |
| `approval::Callback` | 审批决策回调类型别名（`Box<dyn FnOnce(ApproveOrDeny) + Send>`） |
| `Sandbox::spawn` | 启动引擎子进程（fork + exec，等待 socket 就绪） |
| `Sandbox::restart` | 重启引擎子进程 |
| `Sandbox::shutdown` | 关闭引擎子进程 |
| `Sandbox::evaluate` | 通过 IPC 请求引擎评估权限 |
| `Sandbox::reload_rules` | 通过 IPC 重新加载规则 |
| `load_templates_from_dir` | 从目录加载所有 `.json` 模板文件并展开继承链 |
| `Template` / `TemplateSubject` / `load_templates_from_dir` | 模板数据结构和加载接口（见 templates.rs） |
| `closeclaw::permission::engine::engine_risk::assess_risk_level` | 评估请求的风险等级（遍历高危模式列表，engine 子模块内公开） |

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
| `engine/engine_risk.rs` | 风险等级枚举（RiskLevel）、高危模式列表、风险评估函数（assess_risk_level） |
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
| `approval.rs` | 内存审批队列、去重逻辑（基于 SHA256）、审批回调触发 |

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

### 评估算法（9 步）

1. **Creator 规则短路**：若 caller.user_id == agent_creators[agent_id]，直接 Allow
2. **Agent 阶段**：调用 `collect_agent_candidates()` 收集 AgentOnly 候选规则 → `match_rules()` 求值 → 得到 agent_result
3. **Owner 短路**：若 caller.user_id == "owner"，agent_result 即为最终结果，跳过步骤 4~6
4. **User 阶段**：调用 `collect_user_agent_candidates()` 收集 UserAndAgent 候选规则 → `match_rules()` 求值 → 得到 user_result
5. **合并结果**：
   - Agent Deny → Denied
   - User Deny → Denied
   - Agent Allow + User Allow → Allowed
   - Agent Allow + User None → Allowed（向后兼容：仅 Agent 维度有规则，User 维度无立场）
   - Agent None + User Allow → Allowed（向后兼容：仅 User 维度有规则，Agent 维度无立场）
   - Agent None + User None → 默认 Deny
6. **风险评估**：在 `match_rules()` 和 `default_deny()` 返回 Denied 前，调用 `assess_risk_level(request)` 遍历 `HIGH_RISK_PATTERNS`，命中则返回对应等级，否则 Low。风险等级写入 `PermissionResponse::Denied.risk_level`
7. **模板展开**：在 `match_rules()` 内部调用 `expand_templates_sync()` 展开 template 引用为实际 actions
8. **默认策略**：任一阶段返回 None → Denied
9. **Extra Deny 覆盖**：在步骤 1~8 完成后，若 `extra_deny_subjects` 不为空，逐一对每个 subject 调用 `subject.matches(&caller)`，任一匹配则将最终结果覆盖为 `PermissionResponse::Denied`（reason = "action denied by parent agent restriction"，rule = "<extra_deny>"）

### O(1) 索引

引擎内部维护两张索引：
- `agent_rule_index: HashMap<String, Vec<usize>>` — agent_id → 规则下标（**AgentOnly 规则**；UserAndAgent 规则在 `rebuild_indices_with_rules` 时也以 agent 粒度存入同一索引，`collect_agent_candidates()` 会过滤到仅 AgentOnly）
- `user_agent_rule_index: HashMap<String, Vec<usize>>` — `"user_id:agent_id"` → 规则下标（**仅 UserAndAgent 规则**）

查找时先查索引，索引无结果才做 Glob 遍历（仅当该阶段候选列表为空时触发）。
- Agent 阶段：通过 `collect_agent_candidates()` 查 `agent_rule_index`（O(1)），Glob 回退仅在精确匹配无结果时触发
- User 阶段（非 owner）：通过 `collect_user_agent_candidates()` 查 `user_agent_rule_index`（O(1)），Glob 回退仅在精确匹配无结果时触发
- Owner 短路时跳过 `user_agent_rule_index` 查找和 Glob 回退

### 类型继承关系

```
PermissionRequest
├── WithCaller { caller: Caller, request: PermissionRequestBody }
└── Bare(PermissionRequestBody)

RiskLevel（枚举：Low / Medium / High / Critical，Default = Low）

PermissionRequestBody
├── FileOp { path: String, operation: String }
├── CommandExec { command: String, args: Vec<String> }
├── NetOp { operation: String, host: String, port: u16 }
├── ToolCall { skill: String, method: String }
├── InterAgentMsg { agent: String }
└── ConfigWrite { path: String }

PermissionResponse
├── Allowed { token: String }
└── Denied { reason: String, rule: String, risk_level: RiskLevel }

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
    PermissionRequestBody, PermissionResponse, Rule, RuleSet, Subject, TemplateRef,
    RiskLevel };
pub use rules::{ validation, RuleBuilder, RuleSetBuilder,
    RuleBuilderError, RuleSetBuilderError };
```

`approval` 子模块通过 `pub mod approval` 公开，外部通过 `closeclaw::permission::approval::Xxx` 访问，不在顶层 re-export。

`RiskLevel` 从 `engine/engine_risk.rs` 经 `engine::` 重新导出。

`assess_risk_level` 为 `engine` 子模块内公开函数，通过 `closeclaw::permission::engine::engine_risk::assess_risk_level` 访问，未在顶层 re-export。

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
| 模块导出段虚假声称 approval re-exports | 错误 | 文档声称 `pub use approval::{...}` 但 mod.rs 实际无此 re-export，approval 子模块通过 `pub mod approval` 对外可见 |

---

*最后更新：2026-05-15（审批队列 approval 子模块）*
