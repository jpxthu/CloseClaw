# 测试标准（Testing Standards）

> 测试用例的唯一写作规范入口。本文定义测试类型判定、目录组织、命名、fixture、fake LLM、超时、并行安全、临时文件约束、断言风格与单 binary 组织方式。编写任何测试前必读。
>
> 本文只约束测试代码与测试数据，不涉及需求文档（`docs/requirements/`）与设计文档（`docs/design/`）。测试行为对齐 issue 验收标准，不以文档杜撰需求。

## 1. 测试类型判定

一个测试属于 e2e、integration 还是 unit，取决于**它驱动的是哪一层**。核心判据：**看「组合栈是否真实」**。

| 类型 | 定义 | 判定要点 |
|------|------|----------|
| **unit** | 单模块纯逻辑 | 只测单个模块/函数内部逻辑，不跨模块调用，不启动运行时 |
| **integration** | 跨模块直接调用 | 直接调各模块公开 API，mock LLM，**不起完整栈**（不 spawn daemon/sandbox、不拉起完整 session 运行时 + socket） |
| **e2e** | 通过公共入口驱动真实进程组合栈 | spawn 独立 daemon/sandbox 进程（进程外），**或**在进程内拉起完整 session 运行时 + 真实 socket；协议栈真实，LLM 用 fake |

判定顺序：先问「是否 spawn 了独立进程或拉起完整运行时 + 真实 socket」→ 是则 e2e；否则问「是否跨模块调用」→ 是则 integration；否则 unit。

**误放纠正**：单模块测试误放在 `tests/` 根目录平铺的，按 unit 迁回 `crates/<crate>/src/<module>_tests.rs`；跨模块但不起栈的归 integration；起真实进程组合栈的归 e2e。

## 2. 目录结构

```
tests/
├── integration/         # 集成测试（跨模块、不起完整栈）
├── e2e/                 # E2E 测试（真实进程组合栈）
└── fixtures/            # 共享测试数据（死 fixture 见 §4）
crates/<crate>/src/<module>_tests.rs   # 单元测试（与代码同 crate）
```

- cargo 只自动发现 `tests/*.rs`；`tests/integration/` 与 `tests/e2e/` 各自通过单一 `main.rs` 组织（见 §10）。
- `tests/` 根目录**不**平铺单个 `.rs` 测试文件（仅保留 `integration/`、`e2e/`、`fixtures/` 三层）。
- 单元测试与代码分离，不内联 `#[cfg(test)]`（历史遗留除外）。

## 3. 命名规范

| 对象 | 规则 | 示例 |
|------|------|------|
| 测试文件 | `_tests.rs` 复数后缀统一；前缀标明测试类型 | `sigterm_integration_test.rs`、`session_manager_tests.rs` |
| 测试函数 | `test_` 前缀，snake_case | `test_session_compact_on_idle` |
| fixture 目录 | `tests/fixtures/<module>/` | `tests/fixtures/llm/`、`tests/fixtures/feishu/` |

前缀约定（与目录双保险，便于单 binary 内模块可读）：

- e2e 文件用 `e2e_` 或场景主题前缀（如 `sigterm_`、`sandbox_`），后缀 `_test.rs` 或 `_tests.rs` 均可，但**同一目录内统一复数 `_tests.rs`**；
- integration 文件用 `integration_` 或模块主题前缀；
- unit 文件用模块名 `<module>_tests.rs`，无需类型前缀（位置已表达类型）。

> 存量文件迁移时以最小改动为原则，但新增测试必须遵循复数 `_tests.rs` 与类型前缀。

## 4. fixture 存放与引用

- 共享测试数据放 `tests/fixtures/<module>/`，按模块分目录（如 `llm/`、`feishu/`、`outbound/`）。
- 引用一律用 `env!("CARGO_MANIFEST_DIR")` 拼相对路径，**不硬编码绝对路径**：

```rust
let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join("tests/fixtures/feishu");
```

- **死 fixture 不清理**：`llm/`、`outbound/` 下的历史 fixture 已由 issue #2282 跟踪，等 v2 迁移，本次不删除。
- 测试运行时**生成的**临时数据必须走 `tempfile::TempDir`（见 §8），不落 `tests/fixtures/`。

## 5. Fake LLM 强制

- **禁止真实 LLM API 调用**（无网络、无真实 key）。所有测试用进程内 fake provider。
- 使用 `closeclaw_llm::fake::FakeProvider`，通过 Builder 场景驱动：

```rust
use closeclaw_llm::fake::FakeProvider;

let provider = FakeProvider::builder()
    .then_ok("hello", "model-x")          // 成功场景，按 FIFO 消费
    .then_err(ProviderError::Legacy("rate limit".into()))
    .or_else("fallback")                    // 场景耗尽后的兜底（否则 panic）
    .build();
```

- 场景能力：`then_ok` / `then_ok_with` / `then_ok_with_cache` / `then_err` / `then_err_with` / `then_delay` / `or_else` / `stub`。
- 场景按 FIFO 消费，耗尽默认 panic；需要断言「调用被捕获」时用 `captured_requests()` / `drain_requests()` / `clear_requests()`。
- 依赖 fake LLM 的测试用 `#[cfg(feature = "fake-llm")]` 门控，运行时以 `cargo test --features fake-llm` 执行。

## 6. 超时

| 层级 | 超时要求 |
|------|----------|
| 单元测试 | 默认 30s 上限 |
| integration | 参考单测，30s 上限 |
| e2e | 按场景分级：spawn 独立进程的启动/优雅关闭/沙箱创建等场景设置各自合理的更宽松上限，不套统一 30s |

- 30s 是**硬上限**（防挂死），不是目标值。性能目标：CI 中任何 test case 运行超过 5s 必须修复。
- 禁止 `thread::sleep` 等待异步事件（见 §9），等待用 channel/信号量，避免靠超时掩盖竞态。

## 7. 并行安全

- 测试间**不共享可变状态**；不依赖前序测试副作用。
- 涉及端口、文件锁、全局资源（进程全局 env、单例 registry、共享目录）的测试加 `#[serial_test::serial]`。
- **禁止 `std::env::set_var` / `remove_var`**（修改进程全局环境在多线程/并行测试下数据竞争）。配置值通过参数/config struct 传递，读环境用 `std::env::var`（只读）。全库唯一例外是 `daemon/mod.rs` 的 `load_env_file()`。
- 端口不硬编码，用 port 0 让系统分配。

## 8. 临时文件与 config（/tmp 约束）

- 测试使用的 **config 与生成的临时文件必须落在 /tmp**，用 `tempfile::TempDir` 管理，不硬编码路径：

```rust
let dir = tempfile::TempDir::new()?;      // 落在系统 /tmp，Drop 自动清理
```

- 测试后**无残留**进程、端口、临时文件（TempDir 自动清理；spawn 的子进程显式 kill/await）。
- 禁止把临时产物写到仓库目录或 `tests/fixtures/`。

## 9. 断言风格

- 优先精确断言：`assert_eq!`（比较值）、`assert!(cond, "msg")`（带失败信息），少用裸 `assert!(cond)`。
- 失败信息说明「期望什么、实际什么」，便于定位。
- 异步事件等待用 channel / 信号量 / tokio 同步原语，**禁止 `thread::sleep` 轮询**。
- 确定性优先：不依赖时序巧合、不依赖前序测试、不用真实时间做脆弱比较。

## 10. 单 binary 组织方式

cargo 只自动发现 `tests/*.rs`，分层用「每目录一个 binary + `mod` 声明」组织：

```
tests/integration/main.rs    # 声明各集成测试模块
tests/integration/<case>.rs  # 每个测试用例一个文件
tests/e2e/main.rs            # 声明各 e2e 测试模块
tests/e2e/<case>.rs
```

- `main.rs` 只写 `mod <case>;` 声明，不写业务逻辑：

```rust
mod sigterm_integration_test;
mod integration_shutdown_checkpoint;
mod sandbox_integration_test;
```

- 消除现状双编译问题（测试文件内嵌 `mod basic`）与 `#[path]` shim：共享 helper 提取为公共模块或 `tests/fixtures` 下的共享代码，不在单个 case 内重复内嵌 mod。

## 11. 红线

- ❌ 真实 LLM 调用、外部网络访问
- ❌ `thread::sleep` 等待异步事件
- ❌ `std::env::set_var` / `remove_var`
- ❌ 硬编码端口、硬编码临时文件路径
- ❌ 测试后残留进程/端口/文件
- ❌ 依赖前序测试副作用
- ❌ 把 unit 测试放在 `tests/` 根目录平铺
