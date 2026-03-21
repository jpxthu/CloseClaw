# Permission Engine — Overview

## What is the Permission Engine?

The Permission Engine is a **security component** that enforces access-control rules for agents in the CloseClaw multi-agent framework. Before any agent can perform a privileged action — reading a file, executing a command, making a network call, or sending an inter-agent message — the action must be evaluated and explicitly allowed by the engine.

The engine is designed as a **separate OS process** for defense-in-depth: even if the agent runtime is compromised, the engine's strict rule evaluation cannot be bypassed.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        CloseClaw Host Process                    │
│                                                                  │
│  ┌──────────────┐      IPC (Unix socket)      ┌──────────────┐  │
│  │ Agent Runtime │ ◄─────────────────────────► │  Permission  │  │
│  │              │    SandboxRequest/Response    │   Engine     │  │
│  │ - dev-agent  │                              │  (subprocess)│  │
│  │ - prod-agent │                              │              │  │
│  └──────────────┘                              └──────────────┘  │
│                                                              ▲   │
└──────────────────────────────────────────────────────────────┼───┘
                                                               │
                                                  Policy applied:
                                                  - seccomp (Linux)
                                                  - landlock (Linux)
                                                  - deny-by-default
```

### Key Design Decisions

1. **Separate process** — The engine runs as a child process of the host. It is isolated from the agent runtime's memory space and can be killed or restarted independently.
2. **Deny-by-default** — Any action not explicitly allowed by a rule is **denied**. There is no implicit allow.
3. **Stateless evaluation** — Each `evaluate()` call is stateless. The engine reads the ruleset at startup and rebuilds its in-memory index. Rules can be hot-reloaded via the IPC channel.
4. **AWS IAM-style precedence** — When multiple rules match, **deny takes precedence**. If any matching rule says "deny", the action is denied regardless of other allow rules.
5. **O(1) agent lookup** — Rule indices are pre-built into a HashMap keyed by agent ID, giving constant-time lookup for exact matches. Glob patterns fall back to linear scan.

## Components

### `PermissionEngine` (`engine.rs`)

Core rule evaluation logic. Implements AWS IAM–style policy evaluation with:
- `RuleSet` — the full parsed rules document
- `Rule` — a named, tagged rule with a subject, effect, and actions
- `PermissionRequest` / `PermissionResponse` — the evaluation input/output

### `Sandbox` (`sandbox/mod.rs`)

Process lifecycle management for the engine subprocess:
- `spawn()` — starts the engine as a child process
- `restart()` — kills and respawns the engine
- `shutdown()` — cleanly terminates the engine
- `evaluate()` — sends a `PermissionRequest` over IPC and returns a `PermissionResponse`
- `reload_rules()` — hot-reloads the ruleset in the running engine

### `IpcChannel`

Unix domain socket with length-prefixed JSON framing:
```
[4-byte big-endian u32 length][JSON payload]
```

### `SecurityPolicy`

Linux-specific sandbox hardening:
- **seccomp** — restricts available syscalls
- **landlock** — restricts filesystem access paths

## Lifecycle

```
Host starts
    │
    ▼
Sandbox::spawn()
    │
    ├─► spawns engine subprocess (SANDBOX_ENGINE=1)
    │
    ├─► waits for socket to appear
    │
    ├─► sends Ping, awaits Pong
    │
    ▼
Engine is Running ◄──────────────────────┐
    │                                     │
    ├── evaluate(request) ──► Response   │
    ├── reload_rules(ruleset) ──► ACK    │
    │                                     │
    │  (crash detected)                   │
    ▼                                     │
Sandbox::restart() ───────────────────────┘
```

## Serialization

`PermissionRequest` and `PermissionResponse` are serializable with Serde, enabling them to be sent over the IPC channel as JSON.

## Security Notes

- The engine subprocess runs with the **same user privileges** as the host process. For stronger isolation, consider running the engine in a container (Docker/podman) with a seccomp profile and no capabilities.
- Landlock support depends on the kernel version (≥ 5.13). On older kernels the landlock policy is silently skipped.
- Seccomp in this implementation is a **demonstration** — for production use, replace the stub with a proper libseccomp BPF program.
