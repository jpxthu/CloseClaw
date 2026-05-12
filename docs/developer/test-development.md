# 测试开发标准

> 状态：v1 | 2026-05-13

## 目的

本标准约束所有测试代码的安全边界、资源行为和文件组织，确保：
- 测试不会破环环境、留下脏数据或占用端口
- 测试之间无隐式依赖，可并行执行
- 测试资产（fixtures、helpers）在项目中位置一致、可发现

---

## 硬性安全规则

### 资源隔离

| 要求 | 说明 |
|------|------|
| **临时文件/目录** | 所有文件操作必须写入 `tempfile::Builder::new().prefix("closeclaw_test_").tempdir()?` 或 `tempfile::TempDir`，测试结束后自动清理 |
| **端口** | 不得硬编码监听端口；测试服务应让系统分配空闲端口（`port 0` trick） |
| **环境变量** | 不得修改全局环境变量；如需设置用 `env::set_var` 并在测试结束前用 `env::remove_var` 还原 |
| **进程/文件句柄** | 测试退出前确保所有子进程终止、文件句柄关闭；用 `Drop` 或 `abortable` 任务清理 |

### 网络

- **禁止外部网络访问**：所有网络调用必须 mock 或使用本地 mock server
- **禁止长时间阻塞**：单个测试超时上限 30 秒；涉及 I/O 的测试必须设 `#[timeout(XXX)]`
- **禁止真实的 LLM 调用**：使用 `MockLlm` 或 `FakeLlmService`（见 [test/fake-llm](https://github.com/search?q=repo%3Acloseclaw-dev%2Fcloseclaw+label%3Atest%2Ffake-llm)）

### 并行安全

- 测试之间不得共享可变状态（global static、磁盘文件、环境变量）
- `#[tokio::test]` 默认并发；涉及端口/文件锁的测试加 `#[serial]`

---

## 测试分类与文件布局

### 三类测试

| 类型 | 存放位置 | 说明 |
|------|----------|------|
| **UT**（单元测试） | 同文件 `#[cfg(test)]` | 与业务代码同文件，测试内部逻辑 |
| **集成测试** | `tests/` 根目录，按模块名 `*_tests.rs` | 测试模块间交互 |
| **E2E 测试** | `tests/e2e_*.rs` 或 `tests/e2e_<module>/` | 跨模块场景，模拟真实调用链 |

> `src/` 目录下禁止放测试文件（除 `#[cfg(test)]` 模块内）。

### 命名规则

```
tests/
├── e2e_<scenario>_tests.rs          # E2E：一个场景一个文件
├── <module>_tests.rs                # 集成测试：按模块
├── e2e_<module>/
│   ├── mod.rs
│   └── test_case_a.rs              # E2E 子场景拆分
├── fixtures/                        # 共享测试数据
│   ├── mod.rs
│   └── agent_configs.json
└── integration_helpers.rs           # 共享测试辅助函数
```

- 测试模块：`snake_case`，后缀 `_tests`
- 测试函数：`snake_case`，前缀 `test_`（如 `test_session_compact_on_idle`）
- Fixture 文件：与测试模块同名或放 `fixtures/`

---

## UT 标准（同文件测试）

1. 用 `#[cfg(test)]` 模块，不单独拆文件
2. 测试函数 `fn test_<what>() { ... }` 或 `async fn test_<what>()`
3. 不访问网络、不读写磁盘、不启动子进程
4. 用 `#[should_panic]` 时指定 expected message：`#[should_panic(expected = "specific message")]`

---

## 集成测试标准（`tests/`）

1. 所有文件 I/O 必须用 `TempDir`
2. 所有 HTTP/WS 调用走 mock server（`mockito` 或 `wiremock`）
3. 共享 helper 放在 `integration_helpers.rs`，不得复制逻辑
4. 测试前清理：每个测试负责自己的 `TempDir` 退出时自动清理
5. 避免 `lazy_static` / `once_cell` 全局状态

---

## E2E 测试标准

1. 不发真实网络请求；所有外部依赖 mock
2. 完整场景覆盖：启动 → 业务逻辑 → 验证 → 清理
3. 用 `AbortHandle` 保证子进程在测试结束后被 kill
4. 超时：`#[tokio::test(timeout = 60_000)]`
5. 每个 E2E 文件对应一个 GitHub Issue，标签 `test/e2e`

---

## 禁止事项

- ❌ 硬编码 `/tmp/closeclaw_test_xxx` 路径（应用层面 tempfile API）
- ❌ `thread::sleep` 用于等待异步事件（用 `assert_evually` 或 `wait_for`）
- ❌ `unwrap()` 在测试断言外（在测试函数体内可以 `unwrap()` 用于断言失败时的明确 panic）
- ❌ 测试结束后留下进程、端口占用、临时文件
- ❌ 依赖前序测试的副作用（测试必须 self-contained）
- ❌ 访问真实外部网络（飞书 API、Discord API 等）

---

## 现有测试现状

| 类型 | 数量 | 说明 |
|------|------|------|
| 集成测试（`tests/*.rs`） | 20+ | `fake_integration_tests.rs` 等含真实网络依赖，需要改造 |
| E2E | 8+ | `e2e_daemon_*.rs`、`e2e_thinking/` 等，结构较清晰 |
| UT（同文件） | 多个 | 随代码分散 |

---

## 参考

- [code-style.md](./code-style.md) — Rust 编码规范
- `tests/integration_helpers.rs` — 当前共享辅助函数
- `tests/minimax_mock_tests.rs` — Mock LLM 示例