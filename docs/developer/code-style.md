# Rust Code Style Guide

## General Conventions

### Naming
- Types/Traits: `UpperCamelCase` (e.g., `PermissionEngine`)
- Functions/Methods: `snake_case` (e.g., `evaluate_request`)
- Constants: `SCREAMING_SNAKE_CASE` (e.g., `MAX_BUFFER_SIZE`)
- Variables: `snake_case` (e.g., `agent_id`)
- Booleans: `is_` / `has_` / `can_` / `should_` prefix (e.g., `is_ready`, `has_permission`)
- Collections: plural or `_list` / `_map` suffix (e.g., `user_ids`, `config_map`)

### Error Handling
- Use `thiserror` for custom error types with `#[derive(Error)]`
- Use `anyhow` for contextual errors in applications
- Always propagate errors with `?` operator

```rust
// Custom error
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Invalid value for '{field}': {message}")]
    ValueError { field: String, message: String },
};

// Application error
 anyhow::Result<()> {
     read_config().context("Failed to load config")?;
 }
```

### Async Code
- Use `async_trait` for async trait methods
- Always use `.await` after async calls
- Avoid blocking in async context

```rust
#[async_trait]
impl Skill for FileOpsSkill {
    async fn execute(&self, method: &str, args: Value) -> Result<Value, SkillError> {
        // ...
    }
}
```

### Modules
- One module per file
- Use `mod.rs` for module directories
- Export public API with `pub use`

### Documentation
- Document public APIs with `///`
- Use markdown in doc comments
- Include usage examples

```rust
/// Permission engine for evaluating agent requests.
///
/// # Examples
/// ```
/// let engine = PermissionEngine::new(rules);
/// let response = engine.evaluate(request).await;
/// ```
pub struct PermissionEngine { ... }
```

### Testing
- Unit tests in same file with `#[cfg(test)]`
- Integration tests in `tests/` directory
- Use `#[tokio::test]` for async tests
- Use `proptest` for property-based testing

## Key Patterns in CloseClaw

### Builder Pattern
```rust
impl RuleBuilder {
    pub fn new(name: &str) -> Self { ... }
    pub fn with_subject(mut self, subject: Subject) -> Self { ... }
    pub fn with_effect(mut self, effect: Effect) -> Self { ... }
    pub fn build(self) -> Result<Rule, RuleValidationError> { ... }
}
```

### Trait Objects
```rust
// For runtime polymorphism with Send + Sync
Arc<dyn Skill + Send + Sync>

// Use async_trait for async traits
#[async_trait]
pub trait Skill: Send + Sync {
    async fn execute(&self, method: &str, args: Value) -> Result<Value, SkillError>;
}
```

---

## LLM Developer Contract

This project is maintained by AI agents. Every rule below exists because **long contexts are expensive, slow, and error-prone for LLMs**. These are not style preferences — they are architectural constraints.

### Hard Limits

| Metric | Maximum | Rationale |
|--------|---------|-----------|
| File lines | **500** | Beyond this, diff is unreadable and context is wasted |
| Line length | **120 characters** | Enforced by rustfmt |
| Function lines | **50 lines** | Large functions hide complexity |
| Function arguments | **6** | More than 6 signals hidden dependency |
| Module depth | **3 levels** | `crate::foo::bar::baz` is a code smell |
| `impl` block lines | **100** | Split into multiple impl blocks |
| Enum variants | **20** | Beyond this, split into hierarchy |
| Nested match/if | **3 levels** | Beyond this, extract a function |
| `unsafe` blocks | **0 unless documented** | Every unsafe line needs a comment explaining why |

### Project Structure

```
src/
├── main.rs          # Entry point only
├── lib.rs           # Re-exports public API only — no logic
├── mod_a/
│   ├── mod.rs       # Imports + pub use only — no logic
│   ├── a_core.rs    # Core logic < 500 lines
│   └── a_tests.rs   # Tests < 300 lines
```

**Rules:**
- No `inner/` or `private/` subdirectories — structure must be flat and discoverable
- A `mod.rs` must not contain any business logic
- Every file must be independently readable without reading siblings first

### Naming Reinforcements

```rust
// Booleans: mandatory prefix
let is_ready: bool;
let has_permission: bool;
let should_retry: bool;

// Collections: explicit type hint
let user_ids: Vec<u64>;
let config_map: HashMap<String, Value>;

// Errors: specific, not generic
// Bad:  ProcessError, HandleError, DoError
// Good: AuthError, ParseError, ConfigError

// Variables: no abbreviations beyond the obvious
// Bad:  usr, cfg, ctx, msg, buf
// Good: user, config, context, message, buffer
```

### Unsafe Code Policy

Every `unsafe` block **must** have:
1. A comment explaining the invariant that makes this safe
2. A `// Safety:` or `// SAFETY:` prefix on the line above
3. A reference to the Rustonomicon section if applicable

```rust
// SAFETY: The caller guarantees that `ptr` is valid for `len` bytes
// and properly aligned. This is enforced by the factory constructor.
unsafe { std::ptr::read(ptr, len) }
```

---

## Automation Tools

### Formatter: rustfmt

```bash
cargo fmt --check  # CI check
cargo fmt          # auto-fix
```

```toml
# .rustfmt.toml (or pyproject.toml [tool.rustfmt])
edition = "2021"
max_width = 120
```

### Linter: clippy

```bash
cargo clippy --all -- -D warnings  # strict mode
```

```toml
# .clippy.toml or pyproject.toml [tool.clippy]
cyclomatic_complexity = 15
nesting = 3
too_many_arguments = 6
```

### Pre-commit Hook (file line count + lint)

项目自带 `.githooks/pre-commit`，安装方式：

```bash
git config core.hooksPath .githooks
```

Hook 内容（仅检查 staged 文件行数，不背负历史技术债；
cargo fmt 和 clippy 放 CI，因为它们检查整个 crate 无法按文件过滤）：

```bash
#!/bin/bash
set -e

# 仅检查本次提交涉及的文件
git diff --cached --name-only --diff-filter=ACMR -- '*.rs' | while read f; do
    lines=$(wc -l < "$f")
    if [ "$lines" -gt 500 ]; then
        echo "ERROR: $f: ${lines} 行（上限 500）"
        exit 1
    fi
done

echo "All checks passed"
```

### Complexity: cargo-geiger (unsafe tracking)

```bash
cargo install cargo-geiger
cargo geiger [package]  # reports unsafe usage per crate
```

### Memory Safety: miri (nightly only)

```bash
cargo +nightly miri test
```

### TOML: taplo

```bash
cargo install taplo
taplo fmt --check  # validates Cargo.toml and .rustfmt.toml
```

### Recommended CI Pipeline

```yaml
# .github/workflows/ci.yml (example)
steps:
  - name: Format check
    run: cargo fmt --check

  - name: Clippy
    run: cargo clippy --all -- -D warnings

  # CI 检查全量文件（与 pre-commit 不同，CI 要兜底）
  - name: File length check
    run: |
      find src -name "*.rs" -exec wc -l {} + \
        | awk '$1 > 500 { count++ } END { if (count > 0) exit 1 }' \
        && echo "File length check passed"
```
