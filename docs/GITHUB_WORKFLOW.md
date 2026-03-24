# 团队协作流程

> 本文档已迁移至各角色的 AGENTS.md。
> - Dispatcher（调度虾）：`/home/admin/.openclaw/agents/dispatcher/workspace/AGENTS.md`
> - Brainstormer（脑暴虾）：`/home/admin/.openclaw/agents/braino/workspace/AGENTS.md`
> - Planner（计划虾）：`/home/admin/.openclaw/agents/planner/workspace/AGENTS.md`
> - Builder（编码虾）：`/home/admin/.openclaw/agents/builder/workspace/AGENTS.md`
> - Reviewer（审核虾）：`/home/admin/.openclaw/agents/reviewer/workspace/AGENTS.md`
> - Finisher（收尾虾）：`/home/admin/.openclaw/agents/finisher/workspace/AGENTS.md`
> - Debugger（调试虾）：`/home/admin/.openclaw/agents/debugger/workspace/AGENTS.md`
> - Process（流程虾）：`/home/admin/.openclaw/agents/process/workspace/AGENTS.md`

---

## 核心协作约定

所有角色共用同一个 GitHub 账号，通过 Labels 和署名区分职责。
GitHub Issues 是任务的**唯一事实来源**。

### Labels

| Label | 用途 | 谁打 |
|-------|------|------|
| `role:brainstormer` | 待 Brainstormer 领取 | Dispatcher |
| `role:planner` | 待 Planner 领取 | Dispatcher |
| `role:builder` | 待 Builder 领取 | Dispatcher |
| `role:reviewer` | 待 Reviewer 领取 | Dispatcher |
| `role:finisher` | 待 Finisher 领取 | Dispatcher |
| `role:debugger` | 待 Debugger 领取 | Dispatcher |
| `role:process` | 待 Process 领取 | Dispatcher |
| `status:pending` | 待分发 | Dispatcher |
| `status:in-progress` | 进行中 | 领取者 |
| `status:done` | 完成 | 领取者 |
| `bug` / `enhancement` / `documentation` | Issue 类型 | Dispatcher |

### 署名规则

每条 commit / issue comment 末尾署名单独的名字：

```
feat(permission): add user dimension support
— Builder: Alice

docs: add Permission Engine design doc
— Brainstormer: Bob
```

### 任务认领流程

1. Dispatcher 收到任务 → 打 `status:pending` + 对应 `role:*` 标签
2. 对应角色看到自己的标签 → 在 issue 下 comment "我来" → 把 `status:pending` 改成 `status:in-progress`
3. 完成后 → commit → 更新 issue → 改成 `status:done`
4. 没人主动认领 → Dispatcher 直接 assign

---

## 手动验收流程

### 何时需要手动验收

以下类型的 PR/issue 必须经过手动验收：
- **功能实现类**（`feat` 标签）：功能代码变更，需要实际运行验证
- **Bug 修复类**（`bug` 标签）：需要复现问题、验证修复有效
- **工作流程变更类**（`enhancement` 标签）：影响团队协作流程的变更

以下类型的变更**无需手动验收**，CI 通过即可合并：
- **文档类**（`documentation` 标签）：纯文档更新
- **CI/CD 配置类**：GitHub Actions、配置文件等基础设施变更
- **代码重构类**（不改变行为）：仅内部结构优化

### 手动验收检查清单

验收人在 PR 或 issue 下逐项打勾：

```
### 手动验收清单
- [ ] 功能正常工作（描述具体测试了哪些操作）
- [ ] 边界情况已覆盖（如果有）
- [ ] 无新增安全隐患（如果涉及权限/认证）
- [ ] 文档已同步更新（如有必要）
- [ ] 测试通过（`cargo test`）
```

### 验收结果记录

- 在 PR 或 issue 下 comment "✅ 验收通过" 或 "❌ 验收不通过"
- 如不通过，需说明具体问题并 @builder 修复
- 验收通过后，由 Reviewer 或验收人 close issue

### 验收人

| 变更类型 | 验收人 |
|---------|--------|
| 功能实现 | Reviewer（审核虾） |
| Bug 修复 | Reviewer + Reporter（如有） |
| 工作流程变更 | Dispatcher 或指定的 Process 角色 |

---

*迁移时间：2026-03-23*
*最后更新：2026-03-24（角色名称已对齐至新架构）*
