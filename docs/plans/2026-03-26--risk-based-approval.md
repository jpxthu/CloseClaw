# Risk-based Approval — 高风险操作二次确认

## 背景与问题

**现状**：
- Agent 可以直接操作 GitHub（开 issue、merge PR）
- 可以发飞书消息、创建飞书文档
- 没有人工确认环节，Owner 不知道 Agent 做了什么

**需求**：
- 按操作风险分级，中高风险需要 Owner 确认
- 不能因为"等着确认"就卡住所有流程

## 操作风险分级

| 等级 | 操作类型 | 示例 | 处理方式 |
|------|---------|------|---------|
| 🔵 **低风险** | 读操作、写内部文件 | 查资料、读文档、写 memory | 直接执行，无需确认 |
| 🟡 **中风险** | 非破坏性写操作 | 开 issue、评论 PR、创建文档 | 发确认消息，等 Owner 回复再执行 |
| 🔴 **高风险** | 破坏性/不可逆操作 | 删除文件、merge PR、删除数据、撤销 commit | 必须 explicit approve |

## 方案选型

### 方案 A：策略表（Policy Table）

在 config 里声明每个操作的风险等级，运行时查表。

```yaml
approval:
  rules:
    - action: "github.issue.create"
      risk: medium
    - action: "github.pr.merge"
      risk: high
    - action: "feishu.message.send"
      risk: medium
    - action: "file.delete"
      risk: high
```

| 优点 | 缺点 |
|------|------|
| 配置灵活，Owner 可调整 | 需要完整的操作类型枚举 |
| 规则清晰，可审计 | 新操作需要加规则 |

### 方案 B：意图推断（Intent Classification）

让 Agent 在执行前自己声明意图，Gateway 判断是否需要确认。

| 优点 | 缺点 |
|------|------|
| 不需要枚举所有操作 | 模型判断不一定准确 |
| 可以处理未知操作 | 需要额外的 LLM 调用 |

### 方案 C：强制确认（Mandatory Confirm）

所有写操作默认需要确认，Agent 不能自己跳过。

```python
def execute_write(action):
    if action.risk >= MEDIUM:
        pending = create_pending_action(action)
        notify_owner(pending)
        wait_for_approval(pending)
    execute(action)
```

| 优点 | 缺点 |
|------|------|
| 安全，不漏掉任何写操作 | 体验差，低风险操作也要等 |
| 规则简单 |  |

**推荐方案 A + C 混合**：
- 策略表定义风险等级
- 中风险：发飞书确认，执行可并行（Agent 继续做别的）
- 高风险：必须 explicit approve，Agent 等待

## 实现计划

### 步骤一：Action Registry

定义所有可执行的操作及其风险等级。

```python
ACTION_REGISTRY = {
    "github.issue.create": Risk.MEDIUM,
    "github.issue.close": Risk.MEDIUM,
    "github.pr.merge": Risk.HIGH,
    "github.pr.create_review": Risk.LOW,
    "feishu.message.send": Risk.MEDIUM,
    "feishu.doc.create": Risk.MEDIUM,
    "feishu.bitable.create": Risk.MEDIUM,
    "file.write": Risk.LOW,
    "file.delete": Risk.HIGH,
    "file.edit": Risk.LOW,
    "exec.run": Risk.HIGH,  # 执行 shell 命令
}
```

### 步骤二：Approval Service

```python
class ApprovalService:
    def request(self, action: Action) -> PendingApproval:
        risk = ACTION_REGISTRY.get(action.type, Risk.MEDIUM)
        if risk == Risk.LOW:
            return Approved()  # 直接通过
        
        pending = PendingApproval(action, risk)
        if risk == Risk.MEDIUM:
            self.notify_owner_async(pending)  # 发飞书，不等待
        else:  # HIGH
            self.notify_owner_and_wait(pending)  # 发飞书，等待 explicit approve
        return pending
```

### 步骤三：飞书确认消息

```
🤖 [Agent] 请求执行操作：

类型：github.pr.merge
目标：jpxthu/closeclaw PR #94
风险：🔴 高风险

[批准] [拒绝] [查看详情]
```

### 步骤四：Approval 持久化

- 存储在 `~/.closeclaw/approvals/` 目录
- 包含：action 详情、风险等级、Owner 决策、时间戳

## 扩展性

- 支持委托：Owner 可指定"信任某类 Agent"，同类操作自动通过
- 支持时间窗口：紧急情况下，Owner 可设置"未来 1 小时内免确认"
- 支持 audit log：所有操作都有记录，可回溯
