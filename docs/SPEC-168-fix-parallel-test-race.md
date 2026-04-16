# SPEC-168-fix-parallel-test-race

> Date: 2026-04-16
> Status: verify

---

## 1. 背景

`src/system_prompt/` 模块内有 26 个单元测试，在 `cargo test` 默认并行执行时随机失败，串行执行时 100% 通过。

**根因**：`sections.rs:211` 定义的进程级全局状态

```rust
static APPEND_SECTION: RwLock<Option<String>> = RwLock::new(None);
```

`builder.rs` 和 `sections.rs` 的多个测试在测试开始时未清理 `APPEND_SECTION`，且并行测试执行顺序不确定，导致：
- Test A 设置了 `APPEND_SECTION`
- Test B 假设 `APPEND_SECTION` 为空，却读到了 Test A 的脏数据

**受影响测试**（均访问 `APPEND_SECTION`）：
- `test_append_section_not_shown_when_empty` — 假设为空
- `test_build_append_section_appended` — 设置值
- `test_append_section_cleared_after_request` — 假设清空后为空
- `test_append_section_no_truncation`
- `test_append_section_truncation`
- `test_system_command_set_content` — 通过 `handle_system_command` 设置
- `test_system_command_empty_shows_current`
- `test_system_command_truncation`

---

## 2. 修复方案

**选择：给受影响的测试添加 `#[serial]` 属性**

- 使用 `serial_test = "0.1"` crate（dev-dependency）
- 在 `#[test]` 之前叠加 `#[serial]`
- 不修改生产代码（全局状态是正常设计，运行时无竞争）

**备选方案**（未采用）：
- 改为 `Mutex<Option<String>>`：只解决线程安全，不解决测试隔离
- 每个测试用 `setup`/`teardown` 彻底清理全局状态：改动量更大

---

## 3. 变更清单

| 文件 | 变更 |
|------|------|
| `Cargo.toml` | 新增 `serial_test = "0.1"` (dev-dependencies) |
| `src/system_prompt/builder.rs` | `test_append_section_not_shown_when_empty` + `test_build_append_section_appended` + `test_dynamic_sections_not_cached` 加 `#[serial]` |
| `src/system_prompt/sections.rs` | `test_append_section_cleared_after_request` + `test_append_section_no_truncation` + `test_append_section_truncation` 加 `#[serial]` |
| `src/system_prompt/slash_commands.rs` | `test_system_command_set_content` + `test_system_command_empty_shows_current` + `test_system_command_truncation` 加 `#[serial]` |

---

## 4. 验证标准

- [ ] `cargo test system_prompt -- --test-threads=2` 连续 3 次执行均 100% 通过
- [ ] `cargo test system_prompt -- --test-threads=4` 连续 3 次执行均 100% 通过
- [ ] `cargo test system_prompt`（默认并行）连续 5 次执行均 100% 通过
- [ ] `cargo test` 全量测试通过（不破坏其他模块）
