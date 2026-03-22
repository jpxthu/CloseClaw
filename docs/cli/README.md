# CLI Module

> Command-line interface for CloseClaw.

## Commands

### closeclaw agent
```bash
closeclaw agent list              # List all agents
closeclaw agent create <name>     # Create agent
closeclaw agent info <name>      # Show agent details
```

### closeclaw config
```bash
closeclaw config validate <file> # Validate config file
closeclaw config list            # List config files
```

### closeclaw rule
```bash
closeclaw rule check <rule>      # Check rule syntax
closeclaw rule list              # List all rules
```

### closeclaw skill
```bash
closeclaw skill list             # List installed skills
closeclaw skill install <name>   # Install skill
```

### closeclaw chat
```bash
closeclaw chat                    # Start interactive REPL mode
closeclaw chat -m "hello"       # Send single message and exit
```
本地 CLI 直连 daemon（TCP localhost:18889），不依赖 IM 适配器。

### closeclaw run
```bash
closeclaw run --config-dir ./configs  # Start daemon
```

## Global Options
```
-V, --version    Print version
-h, --help       Print help
```

## Examples
```bash
# Validate configuration
closeclaw config validate configs/agents.json

# List all skills
closeclaw skill list

# Start daemon
closeclaw run --config-dir ./configs
```
