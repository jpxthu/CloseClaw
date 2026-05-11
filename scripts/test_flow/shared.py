#!/usr/bin/env python3
"""
Shared utilities for closeclaw integration test scripts.

Provides:
- Wizard PTY interaction helpers
- Daemon process lifecycle management
- Single-shot chat client
"""

import json
import os
import re
import shutil
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Any

import pexpect


# ─────────────────────────────────────────────────────────────────
# Constants
# ─────────────────────────────────────────────────────────────────

REPO = Path(__file__).parent.parent.parent  # closeclaw-test root
BINARY = REPO / "target" / "debug" / "closeclaw"
DEFAULT_CHAT_ADDR = "127.0.0.1:18889"
DAEMON_STARTUP_TIMEOUT = 10  # seconds


# ─────────────────────────────────────────────────────────────────
# Wizard utilities
# ─────────────────────────────────────────────────────────────────

def mask_api_key(content: str) -> str:
    """Mask api_key value in JSON for display."""
    return re.sub(r'"apiKey"\s*:\s*"[^"]+"', '"apiKey": "***"', content)


def run_wizard(
    closeclaw_bin: str | Path,
    api_key: str,
    provider: str = "MiniMax",
) -> dict[str, Any]:
    """
    Run the config wizard interactively in a PTY.

    Returns dict with:
        tmp_home: str — temp directory path (caller cleans up via shutil.rmtree)
        config_home: Path — path to .closeclaw/ directory
        models_json: dict
        creds_json: dict
        minimax_model_ids: set[str]

    NOTE: The temp directory is NOT cleaned up automatically.
    Caller must call ``shutil.rmtree(result["tmp_home"], ignore_errors=True)``
    when done.
    """
    tmp_home = tempfile.mkdtemp(prefix="closeclaw-wizard-")
    env = {**os.environ, "HOME": tmp_home}

    proc = pexpect.spawn(
        str(closeclaw_bin),
        ["config", "setup"],
        encoding="utf-8",
        timeout=60,
        env=env,
        dimensions=(24, 80),
    )

    try:
        # Provider selection — MiniMax is index 0
        proc.expect("Select a provider", timeout=15)
        proc.sendline("0")

        # API token
        proc.expect("API token", timeout=10)
        proc.sendline(api_key)

        # Model selection — may succeed or fallback to knowledge base
        idx = proc.expect(
            [
                "Your selection",   # succeeded or fallback shows list
                "API fetch failed",  # fetch error but still shows list
                "Invalid",          # auth rejected immediately
                pexpect.TIMEOUT,
                pexpect.EOF,
            ],
            timeout=45,
        )
        if idx >= 2:
            proc.terminate()
            raise RuntimeError(
                f"Wizard model selection failed with pexpect index={idx}."
                " Check provider credentials."
            )

        # Select all models
        proc.sendline("all")

        # Confirm
        proc.expect("Confirm?", timeout=10)
        proc.sendline("yes")

        # Wait for write
        proc.expect("Configuration written", timeout=15)

    except Exception as e:
        proc.terminate()
        raise RuntimeError(f"Wizard failed: {e}") from e
    finally:
        proc.close()

    # Verify output files
    config_home = Path(tmp_home) / ".closeclaw"
    models_path = config_home / "config" / "models.json"
    creds_path = config_home / "config" / "credentials" / "minimax.json"

    if not models_path.exists():
        raise FileNotFoundError(f"models.json not written: {models_path}")
    if not creds_path.exists():
        raise FileNotFoundError(f"credentials/minimax.json not written: {creds_path}")

    with open(models_path) as f:
        models_data = json.load(f)
    with open(creds_path) as f:
        creds_data = json.load(f)

    minimax_models = (
        models_data.get("providers", {})
        .get("minimax", {})
        .get("models", [])
    )
    minimax_model_ids = {m["id"] for m in minimax_models}

    return {
        "tmp_home": tmp_home,
        "config_home": config_home,
        "models_json": models_data,
        "creds_json": creds_data,
        "minimax_model_ids": minimax_model_ids,
    }


# ─────────────────────────────────────────────────────────────────
# Daemon process
# ─────────────────────────────────────────────────────────────────

class DaemonProcess:
    """
    Context manager for the closeclaw daemon process.

    The daemon's cwd is set to ``config_home.parent`` (the temp HOME directory),
    so it reads config from ``$HOME/.closeclaw/`` via the standard paths.
    After stop(), the entire temp working directory is cleaned up.
    """

    def __init__(
        self,
        binary: Path,
        config_home: Path,
        log_prefix: str = "daemon",
    ):
        self.binary = binary
        self.config_home = config_home  # Path to .closeclaw/
        self.log_prefix = log_prefix
        self.proc: subprocess.Popen | None = None
        self._log_file = None
        self._log_path: str | None = None
        self._work_dir = str(config_home.parent)  # the temp HOME

    def start(self) -> str:
        """Start daemon, return config_home path."""
        self._log_file = tempfile.NamedTemporaryFile(
            mode="w+",
            prefix=f"{self.log_prefix}-",
            suffix=".log",
            delete=False,
        )
        self._log_path = self._log_file.name
        env = {**os.environ, "HOME": self._work_dir}
        self.proc = subprocess.Popen(
            [str(self.binary), "run"],
            cwd=self._work_dir,
            env=env,
            stdout=self._log_file,
            stderr=subprocess.STDOUT,
        )
        time.sleep(DAEMON_STARTUP_TIMEOUT)
        return str(self.config_home)

    def stop(self):
        """Stop daemon gracefully, then clean up the temp working dir."""
        if self.proc:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()
        if self._log_file:
            self._log_file.close()
        # Remove the entire temp working directory (包含 .closeclaw/)
        if self._work_dir.startswith(tempfile.gettempdir()):
            shutil.rmtree(self._work_dir, ignore_errors=True)

    def is_running(self) -> bool:
        return self.proc is not None and self.proc.poll() is None

    def logs(self) -> str:
        if self._log_path and os.path.exists(self._log_path):
            with open(self._log_path) as f:
                return f.read()
        return ""


# ─────────────────────────────────────────────────────────────────
# Chat client
# ─────────────────────────────────────────────────────────────────

def run_chat(
    binary: Path,
    config_home: Path,
    message: str,
    addr: str = DEFAULT_CHAT_ADDR,
    agent_id: str = "guide",
) -> str:
    """
    Run ``closeclaw chat --message ...`` as a subprocess.
    Returns stdout with INFO-level log lines filtered out.
    """
    env = {**os.environ, "HOME": str(config_home.parent)}
    result = subprocess.run(
        [str(binary), "chat", "--message", message, "--addr", addr, "--agent-id", agent_id],
        cwd=str(config_home.parent),
        env=env,
        capture_output=True,
        text=True,
        timeout=60,
    )
    lines = []
    for line in result.stdout.splitlines():
        # Skip INFO log lines from tracing output
        if re.match(r"\d{4}-\d{2}-\d{2}T[\d:.]+Z\s+\[\d+m\s+INFO", line):
            continue
        lines.append(line)
    return "\n".join(lines).strip()


# ─────────────────────────────────────────────────────────────────
# Build helper
# ─────────────────────────────────────────────────────────────────

def build_binary(repo: Path = REPO) -> Path:
    """Build the closeclaw binary, return Path to it."""
    result = subprocess.run(
        ["cargo", "build", "--bin", "closeclaw"],
        cwd=repo,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"Build failed:\n{result.stderr[-2000:]}")
    return repo / "target" / "debug" / "closeclaw"