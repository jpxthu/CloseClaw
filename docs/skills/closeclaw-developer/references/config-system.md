# Config System — 配置系统、热重载与容错机制

## 配置分块

| 文件 | 内容 |
|------|------|
| `agents.json` | agent 定义、人设、parent/child 关系 |
| `permissions.json` | 权限规则（PE 规则，编译时绑定） |
| `im.json` | IM adapter 配置 |
| `skills.json` | skill 注册和启用 |
| `gateway.json` | 网关配置 |

## 热重载

- **支持热重载**：`agents.json`、`im.json`、`skills.json`
- **不支持热重载**：`permissions.json`（编译时绑定）

## 容错机制

1. 写配置前自动备份上一版本
2. 加载前做 schema 校验
3. 校验失败 → 自动回退到上一可用版本
4. 服务不因配置错误整个挂掉

## ConfigProvider Trait

```rust
trait ConfigProvider {
    fn version(&self) -> Version;
    fn validate(&self) -> Result<()>;
    fn default(&self) -> Self;
    fn rollback(&mut self);
}
```

## 风险

| 问题 | 状态 |
|------|------|
| 配置热重载原子性 | Phase 9 |
