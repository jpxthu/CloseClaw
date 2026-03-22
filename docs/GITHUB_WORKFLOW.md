# 团队协作流程

> 本文档已迁移至各角色的 AGENTS.md。
> - Dispatcher（Vibe虾）：`/home/admin/.openclaw/agents/vibe/workspace/AGENTS.md`
> - Designer：`/home/admin/.openclaw/agents/designer/workspace/AGENTS.md`
> - Coder：`/home/admin/.openclaw/agents/coder/workspace/AGENTS.md`
> - Tester：`/home/admin/.openclaw/agents/tester/workspace/AGENTS.md`
> - Process Owner：`/home/admin/.openclaw/agents/process/workspace/AGENTS.md`

---

## 核心协作约定

所有角色共用同一个 GitHub 账号，通过 Labels 和署名区分职责。
GitHub Issues 是任务的**唯一事实来源**。

### Labels

| Label | 用途 | 谁打 |
|-------|------|------|
| `role:designer` | 待 Designer 领取 | Dispatcher |
| `role:coder` | 待 Coder 领取 | Dispatcher |
| `role:tester` | 待 Test Specialist 领取 | Dispatcher |
| `role:process` | 待 Process Owner 领取 | Dispatcher |
| `status:pending` | 待分发 | Dispatcher |
| `status:in-progress` | 进行中 | 领取者 |
| `status:done` | 完成 | 领取者 |
| `bug` / `enhancement` / `documentation` | Issue 类型 | Dispatcher |

### 署名规则

每条 commit / issue comment 末尾署名单独的名字：

```
feat(permission): add user dimension support
— Coder: Alice

docs: add Permission Engine design doc
— Designer: Bob
```

### 任务认领流程

1. Dispatcher 收到任务 → 打 `status:pending` + 对应 `role:*` 标签
2. 对应角色看到自己的标签 → 在 issue 下 comment "我来" → 把 `status:pending` 改成 `status:in-progress`
3. 完成后 → commit → 更新 issue → 改成 `status:done`
4. 没人主动认领 → Dispatcher 直接 assign

---

*迁移时间：2026-03-23*
