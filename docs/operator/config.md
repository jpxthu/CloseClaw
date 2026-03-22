# Configuration Guide

## Configuration Files

```
configs/
├── agents.json          # Agent definitions
├── permissions.json     # Permission rules
├── skills.json         # Skill registry (optional)
└── .env               # Environment variables (API keys)
```

## agents.json

```json
{
  "version": "1.0",
  "agents": [
    {
      "name": "agent-name",
      "model": "provider/model-name",
      "persona": "Agent role description",
      "parent": "parent-agent-name",
      "max_iterations": 100,
      "timeout_minutes": 30,
      "skills": ["file_ops", "git_ops"]
    }
  ]
}
```

| Field | Required | Description |
|-------|----------|-------------|
| `name` | Yes | Unique agent identifier |
| `model` | Yes | LLM model (e.g., `minimax/MiniMax-M2.7`) |
| `persona` | No | Agent personality/role description |
| `parent` | No | Parent agent for inheritance |
| `max_iterations` | No | Max conversation turns |
| `timeout_minutes` | No | Conversation timeout |
| `skills` | No | Enabled skills |

## .env Configuration

```bash
MINIMAX_API_KEY=your_api_key_here
OPENAI_API_KEY=your_api_key_here
FEISHU_WEBHOOK=https://...
```

## Hot Reload
- Changes to `agents.json` and `skills.json` are reloaded automatically
- `permissions.json` requires restart (compile-time binding)
- Backup is created automatically before changes

## Validation
```bash
closeclaw config validate configs/agents.json
```
