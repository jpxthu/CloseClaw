# CLI Command Reference

## closeclaw Commands

### Global Options
```
-V, --version    Print version
-h, --help       Print help
```

### Agent Subcommand
```bash
closeclaw agent list                    # List all agents
closeclaw agent create <name>          # Create agent
closeclaw agent create <name> -m <model>  # Create with model
closeclaw agent info <name>            # Show agent details
```

### Config Subcommand
```bash
closeclaw config validate <file>      # Validate config file
closeclaw config list                 # List config files
```

### Rule Subcommand
```bash
closeclaw rule check <rule>          # Check rule syntax
closeclaw rule list                    # List all rules
```

### Skill Subcommand
```bash
closeclaw skill list                  # List installed skills
closeclaw skill install <name>         # Install a skill
```

### Run Daemon
```bash
closeclaw run --config-dir ./configs  # Start daemon
```

## Examples

### Validate Configuration
```bash
closeclaw config validate ./configs/agents.json
closeclaw config validate ./configs/permissions.json
```

### Check Permission Rules
```bash
closeclaw rule check "dev-agent-file-read"
```

### List Available Skills
```bash
closeclaw skill list
# Output:
# Installed skills:
#   file_ops v1.0.0
#   git_ops v1.0.0
#   search v1.0.0
```
