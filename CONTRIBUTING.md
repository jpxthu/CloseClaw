# 开发贡献指南

> CloseClaw 项目的唯一开发规范入口。编码、测试、Git、PR 流程都在这里。

## 前置条件

- Rust 工具链（rustup 安装最新 stable）
- GitHub CLI（`gh`），已登录：`gh auth login`
- Git Hook：`git config core.hooksPath .githooks`

## 文档与代码原则

**一个概念只在一处完整定义，其他地方引用即可。**

---

## 快速开始

```bash
git clone git@github.com:jpxthu/CloseClaw.git
cd CloseClaw
cargo build && cargo test
```

环境搭建详见 [docs/SETUP.md](docs/SETUP.md)。

---

## 代码风格

### 命名

| 类别 | 规则 | 示例 |
|------|------|------|
| 类型/Trait | `UpperCamelCase` | `PermissionEngine` |
| 函数/方法 | `snake_case` | `evaluate_request` |
| 常量 | `SCREAMING_SNAKE_CASE` | `MAX_BUFFER_SIZE` |
| 布尔值 | `is_` / `has_` / `can_` / `should_` 前缀 | `is_ready` |
| 集合 | 复数或 `_list` / `_map` 后缀 | `user_ids` |
| 模块/目录 | `snake_case`，单一单词优先 | `system_prompt` |

可见性：模块内部用 `pub(crate)`，仅对外暴露的 API 用 `pub`。

### 错误处理

- 自定义错误用 `thiserror` + `#[derive(Error)]`
- 上下文错误用 `anyhow::Context`
- 统一用 `?` 传播

### 异步代码

- trait 异步方法用 `#[async_trait]`
- 禁止在 async 上下文中阻塞

### 模块

- 一个文件一个模块，`mod.rs` 只放 `pub use` / `pub mod`
- 公共 API 用 `pub use` 导出
- 公开 API 用 `///` 文档注释

---

## 硬性限制

以下限制中，文件行数由 pre-commit hook 检查；其余由 CI 强制执行。

| 指标 | 上限 |
|------|------|
| 文件行数（含测试） | **1000** |
| 单行宽度 | **100 字符** |
| 函数体行数 | **50** |
| 函数参数 | **6** |
| 模块嵌套深度 | **3 层**（`crate::a::b`，不含 crate 自身） |
| impl 块行数 | **100** |
| enum 变体数 | **20** |
| 嵌套 match/if | **3 层** |
| unsafe 块 | **0（除非注释说明）** |
| `std::env::set_var` / `remove_var` | **禁止**（`load_env_file` 除外） |

---

## 项目结构

CloseClaw 采用 Cargo workspace，每个 design doc 模块对应一个独立 crate。

### 依赖分层

```
Layer 0（叶子）：closeclaw-common
Layer 1：closeclaw-config, closeclaw-session
Layer 2：closeclaw-llm, closeclaw-permission
Layer 3：closeclaw-gateway, closeclaw-tools, closeclaw-skills, closeclaw-im-adapter
Layer 4：closeclaw-agent, closeclaw-memory, closeclaw-processor-chain, closeclaw-slash, closeclaw-system-prompt, closeclaw-platform, closeclaw-tasks
Layer 5：closeclaw-cli, closeclaw-daemon
Layer 6：closeclaw（主 crate，纯组合根，仅 main.rs + lib.rs）
```

**规则**：
- 跨模块共享的类型和 trait 放 `closeclaw-common`
- 各 crate 内部模块嵌套不超过 4 级目录
- 每个文件独立可读
- 禁止逆向依赖（高层 → 低层允许，反向禁止）
- 循环依赖通过 trait 抽象解耦

### 目标目录结构

```
Cargo.toml               # workspace 根
src/                     # 主 crate（closeclaw）— 纯组合根
├── lib.rs               #   pub mod + init()
├── main.rs              #   bin 入口
├── bridge.rs            #   模块间桥接（临时，逐步消除）
└── common/              #   共享类型（逐步下沉到 closeclaw-common）
crates/
├── common/              # closeclaw-common — 共享类型、trait、常量
├── config/              # closeclaw-config — 配置管理、热重载
├── session/             # closeclaw-session — 持久化、bootstrap、recovery、compaction
├── llm/                 # closeclaw-llm — 多供应商 LLM 抽象
├── permission/          # closeclaw-permission — 权限引擎、审批流
├── gateway/             # closeclaw-gateway — 消息路由、IM 插件管理
├── tools/               # closeclaw-tools — 工具注册、内置工具
├── skills/              # closeclaw-skills — skill 注册、加载
├── im_adapter/          # closeclaw-im-adapter — IM 平台适配
├── agent/               # closeclaw-agent — Agent 配置档案、注册表、spawn
├── memory/              # closeclaw-memory — 长期记忆（mining、dreaming、active search）
├── processor_chain/     # closeclaw-processor-chain — 出入站消息处理链
├── slash/               # closeclaw-slash — 斜杠指令系统
├── system_prompt/       # closeclaw-system-prompt — System Prompt 构建
├── platform/            # closeclaw-platform — OS 抽象层
├── tasks/               # closeclaw-tasks — 后台任务管理
├── cli/                 # closeclaw-cli — 命令行接口
└── daemon/              # closeclaw-daemon — 进程入口、启动编排、优雅关闭
tests/
├── integration/         # 集成测试
├── e2e/                 # E2E 测试
└── fixtures/            # 共享测试数据
```

### Crate 拆分标准

**拆分原则**：
- 一个 design doc 模块 = 一个 crate
- 根 crate 最终只保留 main.rs + lib.rs（纯组合根）
- 每次只拆分一个模块，一个 PR
- 每个 PR 必须通过 `cargo build` + `cargo test`
- 禁止修改 design doc

**拆分流程**：
1. 创建 `refactor/<module>` 分支
2. 将 `src/<module>/` 移动到 `crates/<module>/src/`
3. 创建 `crates/<module>/Cargo.toml`，声明最小依赖集
4. 更新 workspace `members` 和根 crate 依赖
5. 处理跨模块依赖（类型下沉 / trait 抽象 / pub use 桥接）
6. `cargo build && cargo test` 通过
7. 按 Git 工作流提 PR

**跨模块依赖处理**：
- 共享类型 → 下沉到 `closeclaw-common`
- 双向依赖 → 通过 trait 抽象解耦，低层 crate 定义 trait，高层 crate 实现
- 过渡期允许 `pub use` re-export 桥接，后续 PR 消除

---

## unsafe 代码

每个 `unsafe` 块必须具备：
1. `// SAFETY:` 前缀的注释说明不变量
2. 如适用，引用 [Rustonomicon](https://doc.rust-lang.org/nomicon/) 对应章节

---

## 测试

### 硬性安全规则

| 要求 | 说明 |
|------|------|
| **临时文件/目录** | 用 `tempfile::TempDir`，不可硬编码路径 |
| **端口** | 不硬编码，用 port 0 系统分配 |
| **环境变量** | 禁止 `std::env::set_var` / `remove_var`，详见下方「环境变量禁令」 |
| **网络** | 禁止外部网络访问，全部 mock |
| **超时** | 单测 30s |
| **LLM** | 禁止真实 LLM 调用 |
| **并行安全** | 测试间不共享可变状态；涉及端口/文件锁加 `#[serial_test::serial]` |

### 环境变量禁令

`std::env::set_var` / `std::env::remove_var` 修改进程全局环境，在多线程和并行测试中会导致数据竞争。**全代码库禁止使用**，唯一例外是 `daemon/mod.rs` 中的 `load_env_file()`（启动阶段加载 `.env` 文件）。

正确做法：
- 配置值通过参数/config struct 传递，不写入全局 env
- 测试需要隔离配置时，用依赖注入或临时文件路径，不用 `set_var`
- 需要读取环境变量时用 `std::env::var`（只读，安全）

违反此规则的 commit 会被 pre-commit hook 和 CI 拦截。

### 布局

| 类型 | 位置 |
|------|------|
| 单元测试 | `crates/<crate>/src/<module>_tests.rs` |
| 集成测试 | `tests/integration/` |
| E2E | `tests/e2e/` |

> UT 与代码分离，不在功能文件中内联 `#[cfg(test)]`。已有内联测试属于历史遗留。

### 命名

| 对象 | 规则 | 示例 |
|------|------|------|
| 测试文件 | `_tests.rs` 后缀 | `session_manager_tests.rs` |
| 测试函数 | `test_` 前缀 | `test_session_compact_on_idle` |
| Fixture | `tests/fixtures/<module>/` | `tests/fixtures/llm/` |

### 禁止事项

- ❌ `thread::sleep` 等待异步事件
- ❌ 测试后残留进程、端口、临时文件
- ❌ 依赖前序测试的副作用
- ❌ 访问真实外部网络
- ❌ UT 中出现 >1s 的等待

### 性能约束

- 单测中禁止出现 >1s 的等待（sleep、timeout、阻塞 IO）
- CI 中任何 test case 运行超过 5s 必须修复

---

## Git 工作流

### 红线

- 禁止直接 push master，所有变更通过 PR squash merge
- 禁止 force push master
- master 保持线性历史

### 分支与 Commit 类型

| 分支前缀 | Commit Type | 用途 |
|---------|-------------|------|
| `feat/` | `feat` | 新功能 |
| `fix/` | `fix` | Bug 修复 |
| `docs/` | `docs` | 文档变更 |
| `refactor/` | `refactor` | 重构 |
| `test/` | `test` | 测试 |
| `perf/` | `perf` | 性能优化 |
| `chore/` | `chore` | 维护 |

分支名和 commit type 必须对应。例如 `feat/xxx` 分支的 commit 用 `feat`。

### Commit Footer

Commit 和 PR body 末尾都必须包含：

```
Source: issue #N
Type: <type>
```

| Footer | 含义 | 可选值 |
|--------|------|--------|
| `Source:` | 变更来源 | `issue #N` / `CI` / `user` |
| `Type:` | 变更类型 | 见上方分支与 Commit 类型表 |

CI 会校验这两个 footer，缺失则阻止合并。

### 标准开发流程

```bash
# 1. 拉最新，创建分支
git checkout master && git pull
git checkout -b <prefix>/<name>

# 2. 开发 + 预检
cargo fmt && cargo clippy -- -D warnings && cargo test

# 3. 提交
git commit -m "<type>: 简述

Source: issue #N
Type: <type>"

# 4. 推送
git push -u origin <prefix>/<name>
```

### PR 与 Merge

```bash
# 准备 PR body（写入文件，PR body = squash merge 后的 commit body）
cat > /tmp/pr-body.md <<'EOF'
PR 概述（做了什么、为什么）

Source: issue #N
Type: <type>
EOF

# 创建 PR
gh pr create --title "<type>: 简述" --body-file /tmp/pr-body.md

# 合并（review 通过后）
gh pr merge --squash --delete-branch --body-file /tmp/pr-body.md

# 更新本地
git checkout master && git pull
```

> `--body-file` 确保 PR body 准确传递为 squash commit body。`--delete-branch` 同时删除远程和本地分支。不用 `--subject`——PR title 自动成为 commit subject。

---

## 工具链

```bash
cargo fmt --check    # CI 格式检查
cargo fmt            # 自动修复
cargo clippy --all -- -D warnings
cargo test
```

---

## 相关文档

| 文档 | 说明 |
|------|------|
| [docs/SETUP.md](docs/SETUP.md) | 环境搭建 |
| [docs/design/](docs/design/README.md) | 模块设计文档 |
