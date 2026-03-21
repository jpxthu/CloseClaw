# CloseClaw Skill Creator

> Guide for creating new skills for CloseClaw.

## Overview
Skills are pluggable modules that extend CloseClaw's capabilities.

## Topics
- [overview.md](overview.md) - Skill architecture
- [tutorial.md](tutorial.md) - Step-by-step guide
- [best-practices.md](best-practices.md) - Best practices

## Quick Reference
```rust
// Implement Skill trait
#[async_trait]
impl Skill for MySkill {
    fn manifest(&self) -> SkillManifest { ... }
    fn methods(&self) -> Vec<&str> { ... }
    async fn execute(&self, method: &str, args: Value) -> Result<Value, SkillError> { ... }
}
```

## Skill Locations
- Built-in: `src/skills/*.rs` (compiled into binary)
- External: `~/.closeclaw/skills/` (loaded at runtime)
