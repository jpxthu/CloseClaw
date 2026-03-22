# Permission Rules — 完整规则格式与 PE 接口

> 权限引擎（PE）是 CloseClaw 的核心创新，所有 agent 操作必须经过它审查。

## 规则格式（JSON）

```json
{
  "version": "1.0",
  "rules": [
    {
      "name": "dev-agent-file-read",
      "subject": { "agent": "dev-agent-01" },
      "effect": "allow",
      "actions": [
        {
          "type": "file",
          "operation": "read",
          "paths": ["/home/admin/code/**"]
        },
        {
          "type": "file",
          "operation": "write",
          "paths": ["/home/admin/code/closeclaw/src/**"]
        }
      ]
    },
    {
      "name": "dev-agent-git",
      "subject": { "agent": "dev-agent-01" },
      "effect": "allow",
      "actions": [
        {
          "type": "command",
          "command": "git",
          "args": { "allowed": ["status", "log", "diff", "add", "commit", "push", "pull"] }
        }
      ]
    }
  ],
  "defaults": { "file": "deny", "command": "deny", "network": "deny", "inter_agent": "deny", "config": "deny" }
}
```

## 字段说明

| 字段 | 说明 |
|------|------|
| `name` | 规则唯一标识 |
| `subject` | 目标 agent，支持精确匹配或 glob |
| `effect` | `allow` 或 `deny`（deny 优先） |
| `actions[].type` | 操作类型：`file`、`command`、`network`、`tool`、`inter_agent`、`config` |
| `actions[].operation` | 文件操作：`read`、`write`、`exec` |
| `actions[].paths` | 路径列表，支持 glob |
| `actions[].args` | 命令参数：`allowed`（白名单）或 `blocked`（黑名单） |

## 评估逻辑

1. 请求到达 PE → 解析 action 类型
2. 匹配 `subject`（精确或 glob）
3. 按优先级评估 `effect`
4. **`deny` 优先于 `allow`**（AWS IAM 风格）
5. 未匹配 → `defaults` 裁定

## Permission Engine 接口

```rust
pub enum PermissionRequest {
    FileOp { agent: String, path: String, op: FileOp },
    CommandExec { agent: String, cmd: String, args: Vec<String> },
    NetOp { agent: String, host: String, port: u16 },
    ToolCall { agent: String, skill: String, method: String },
    InterAgentMsg { from: String, to: String },
    ConfigWrite { agent: String, config_file: String },
}

pub enum PermissionResponse {
    Allowed { token: String },
    Denied { reason: String, rule: String },
}
```

## 风险与开放问题

| 问题 | 状态 |
|------|------|
| landlock 对容器环境要求 | 待确认（需内核 5.13+） |
| seccomp 规则粒度 | 待定 |
| Windows Sandbox 支持 | 待实现 |
