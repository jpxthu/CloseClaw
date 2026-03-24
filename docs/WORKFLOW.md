# CloseClaw Development Workflow

## Skill Maintenance Rule
> **Important**: When modifying or adding code, always update corresponding SKILL.md documentation.

## Skill Deployment Model

### Built-in Skills
Skills in `src/skills/*.rs` are compiled into the binary.
- Always available, no deployment needed
- Listed in `src/skills/builtin.rs` -> `builtin_skills()`
- Tests co-located in the same `.rs` files

### Documentation Skills
SKILL.md files in `docs/skill-creator/`、`docs/developer/`、`docs/operator/` are loaded by the agent at runtime from the filesystem — they are **not** compiled into the binary.
- Agent reads these at runtime to understand how to use skills
- Must be manually kept in sync with code

## Development Workflow

### When Adding a Skill
1. Implement in `src/skills/my_skill.rs`
2. Add to `builtin_skills()` in `src/skills/builtin.rs`
3. Write tests in same file
4. Create `docs/<category>/my_skill/SKILL.md` (e.g. `docs/developer/` or `docs/operator/`)
5. Run `cargo test --lib`
6. Commit

### When Modifying a Skill
1. Modify code in `src/skills/`
2. Update tests
3. Update SKILL.md documentation
4. Ensure `cargo test` passes
5. Commit

### When Adding a Module
1. Implement in `src/module_name/`
2. Add tests
3. Create `docs/module_name/README.md`
4. Update this workflow if needed
5. Commit

## Build Process
```bash
cargo build --release
# Built-in skills are compiled in automatically
# SKILL.md files should be distributed with binary
```
