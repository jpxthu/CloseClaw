# Troubleshooting Guide

## Common Issues

### Agent Not Responding
1. Check agent state: `closeclaw agent info <name>`
2. Check logs for errors
3. Verify agent has required skills
4. Check permission rules allow the request

### Permission Denied
1. Review `permissions.json` for matching rules
2. Check subject glob patterns
3. Verify action type matches request
4. Remember: deny rules take precedence

### Config Validation Failed
```bash
closeclaw config validate configs/agents.json
```
- Check JSON syntax
- Verify required fields present
- Ensure version format is correct

### Permission Rule Check Failed
- Verify rule JSON structure
- Check glob patterns are valid
- Ensure action types are correct

## Log Format
```
2026-03-21 10:30:00 [INFO] closeclaw: Agent vibe initialized
2026-03-21 10:30:05 [INFO] closeclaw: Permission request from vibe for file/read
2026-03-21 10:30:05 [DEBUG] closeclaw: Rule match: dev-agent-file-read (allow)
2026-03-21 10:30:05 [INFO] closeclaw: Request allowed
```

## Debug Mode
```bash
RUST_LOG=debug cargo run -- run --config-dir ./configs
```

## Backup and Rollback
- Backups are stored in `.closeclaw/backups/`
- Rollback: `closeclaw config rollback <file>`
- List backups: `ls .closeclaw/backups/`
