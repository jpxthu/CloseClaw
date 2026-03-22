---
name: closeclaw-operator
description: |
  CloseClaw 运维指南。Use when: (1) configuring or managing a CloseClaw instance, (2) setting up agents, permissions, or IM adapters, (3) troubleshooting CloseClaw issues, (4) running closeclaw CLI commands (agent list, config validate, etc.), (5) understanding config files or permission rules in production.
---

# CloseClaw Operator Skill

> Guide for operators managing CloseClaw instances.

## Quick Reference
| Task | Command |
|------|---------|
| List agents | `closeclaw agent list` |
| Validate config | `closeclaw config validate <file>` |
| Check rules | `closeclaw rule check <rule>` |
| List skills | `closeclaw skill list` |
| Start daemon | `closeclaw run --config-dir ./configs` |
| Stop daemon | `closeclaw stop` |

## Topics
- [cli.md](cli.md) — CLI command reference
- [config.md](config.md) — Configuration guide
- [permissions.md](permissions.md) — Permission rules
- [troubleshooting.md](troubleshooting.md) — Common issues

## 环境与构建

详细环境配置见 `docs/SETUP.md`。

## 配置文件参考

配置系统完整说明见 `docs/config/README.md`。

## 故障排除

常见问题见 [troubleshooting.md](troubleshooting.md)。
