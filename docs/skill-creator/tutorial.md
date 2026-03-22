# Skill Creation Tutorial

## Step 1: Create Skill File
```bash
touch src/skills/my_skill.rs
```

## Step 2: Implement Skill Trait
```rust
use async_trait::async_trait;
use crate::skills::{Skill, SkillManifest, SkillError};

pub struct MySkill;

impl MySkill {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl Skill for MySkill {
    fn manifest(&self) -> SkillManifest {
        SkillManifest {
            name: "my_skill".to_string(),
            version: "1.0.0".to_string(),
            description: "What my skill does".to_string(),
            author: Some("Your Name".to_string()),
            dependencies: vec![],
        }
    }

    fn methods(&self) -> Vec<&str> {
        vec!["do_something", "check_status"]
    }

    async fn execute(&self, method: &str, args: Value) -> Result<Value, SkillError> {
        match method {
            "do_something" => self.do_something(args).await,
            "check_status" => self.check_status().await,
            _ => Err(SkillError::MethodNotFound {
                skill: "my_skill".to_string(),
                method: method.to_string(),
            })
        }
    }
}

impl MySkill {
    async fn do_something(&self, args: Value) -> Result<Value, SkillError> {
        let input = args.get("input")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SkillError::InvalidArgs("input required".to_string()))?;
        
        // Implementation here
        Ok(serde_json::json!({ "result": "done" }))
    }

    async fn check_status(&self) -> Result<Value, SkillError> {
        Ok(serde_json::json!({ "status": "ok" }))
    }
}
```

## Step 3: Register in mod.rs
```rust
pub mod my_skill;
pub use my_skill::MySkill;
```

## Step 4: Add to Built-in Skills
In `src/skills/builtin.rs`:
```rust
pub fn all() -> Vec<Arc<dyn Skill>> {
    vec![
        // ... existing skills ...
        Arc::new(MySkill::new()),
    ]
}
```

## Step 5: Test
```rust
#[tokio::test]
async fn test_my_skill() {
    let skill = MySkill::new();
    let result = skill.execute("check_status", json!({})).await;
    assert!(result.is_ok());
}
```

## Step 6: Document
Create `docs/skills/my_skill/SKILL.md` following the standard format.
