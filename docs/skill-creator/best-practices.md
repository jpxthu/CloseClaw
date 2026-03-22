# Skill Best Practices

## 1. Input Validation
Always validate inputs and return clear error messages.
```rust
let required_field = args.get("field")
    .and_then(|v| v.as_str())
    .ok_or_else(|| SkillError::InvalidArgs(
        "field 'field' is required".to_string()
    ))?;
```

## 2. Error Messages
Use descriptive error messages that help debugging.
```rust
Err(SkillError::ExecutionFailed(
    format!("Failed to read file '{}': {}", path, e)
))
```

## 3. Async Execution
Don't block in async context. Use async I/O.
```rust
// Good
tokio::fs::read(path).await?

// Avoid
std::fs::read(path)?  // Blocks the thread
```

## 4. Version Management
Follow semver. Document breaking changes.
```rust
// 1.0.0 -> 1.1.0 (additive)
// 1.0.0 -> 2.0.0 (breaking)
```

## 5. Skill Isolation
Each skill should be independent. Don't couple skills.
```rust
// Good: Skill has its own dependencies
// Avoid: Skills calling each other directly
```

## 6. Documentation
Document every method with examples.
```rust
/// Execute a search query.
///
/// # Arguments
/// * `query` - The search query string
///
/// # Returns
/// * `results` - Array of search results
///
/// # Example
/// ```rust
/// skill.execute("search", json!({"query": "rust"})).await?;
/// ```
```

## 7. Testing
Test each method thoroughly, including error paths.
```rust
#[tokio::test]
async fn test_invalid_input() {
    let skill = MySkill::new();
    let result = skill.execute("method", json!({})).await;
    assert!(result.is_err());
}
```

## 8. Performance
- Use connection pooling for external services
- Cache results when appropriate
- Don't load large data into memory unnecessarily

## 9. Security
- Validate all inputs
- Don't expose sensitive data in errors
- Respect permission boundaries

## 10. Lifecycle
1. Implement skill
2. Write tests
3. Document methods
4. Update SKILL.md
5. Add to registry
