# CloseClaw 快速上手

> 你的第一个安全的 AI 助手

## 🚀 立即开始

### 1. 配置 API Key

```bash
cd /home/admin/code/closeclaw
closeclaw config setup
```

### 2. 启动引导模式

```bash
cargo run -- run --config-dir ./configs
```

### 3. 和 guide agent 对话

guide agent 会引导你完成后续配置。

---

## 🔒 安全设计

guide agent 默认权限：

| 权限 | 状态 |
|------|------|
| 读取源码 (.rs, .md, .toml) | ✅ 允许 |
| 读取公开文件 (README, LICENSE) | ✅ 允许 |
| 搜索功能 | ✅ 允许 |
| **写入任何文件** | ❌ 禁止 |
| **读取配置文件** (.env, api_key) | ❌ 禁止 |
| **执行命令** | ❌ 禁止 |
| **网络请求** | ❌ 禁止 |

---

## 📁 配置文件说明

```
configs/
├── agents.json           # Agent 定义（guide agent 已预配置）
├── permissions.json      # 权限规则（安全策略已配置）
├── .env.example          # API Key 配置模板
└── skills.json          # Skill 注册（可选）
```

---

## ➕ 添加更多 Agent

编辑 `configs/agents.json` 添加新 agent：

```json
{
  "name": "my-agent",
  "model": "minimax/MiniMax-M2.7",
  "persona": "你的角色描述",
  "parent": "guide"
}
```

然后在 `configs/permissions.json` 中添加对应的权限规则。

---

## ❓ 获取帮助

- 查看源码：`./closeclaw agent list`
- 查看 Skills：`./closeclaw skill list`
- 查看文档：[SPEC.md](./SPEC.md)
