# Rust Code Style Guide

## General Conventions

### Naming
- Types/Traits: `UpperCamelCase` (e.g., `PermissionEngine`)
- Functions/Methods: `snake_case` (e.g., `evaluate_request`)
- Constants: `SCREAMING_SNAKE_CASE` (e.g., `MAX_BUFFER_SIZE`)
- Variables: `snake_case` (e.g., `agent_id`)

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
}

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
