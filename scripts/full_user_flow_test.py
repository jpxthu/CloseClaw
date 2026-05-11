#!/usr/bin/env python3
"""
完整用户流程测试脚本：config wizard → CLI chat agent 对话

用法：
    python3 scripts/full_user_flow_test.py [--api-key KEY] [--wizard-only] [--chat-only]

流程：
    Step 1: config wizard (交互式 PTY)
    Step 2: 启动 daemon (后台)
    Step 3: CLI chat 对话 (单次消息模式)

依赖：pexpect (pip install pexpect)
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import pexpect

# ─────────────────────────────────────────────────────────────────
# 配置
# ─────────────────────────────────────────────────────────────────

REPO = Path(__file__).parent.parent
BINARY = REPO / "target" / "debug" / "closeclaw"
DEFAULT_CHAT_ADDR = "127.0.0.1:18889"
DAEMON_STARTUP_TIMEOUT = 10  # seconds


# ─────────────────────────────────────────────────────────────────
# Step 1: Config Wizard
# ─────────────────────────────────────────────────────────────────

def mask_api_key(content: str) -> str:
    """Mask api_key value in JSON for display."""
    return re.sub(r'"apiKey"\s*:\s*"[^"]+"', '"apiKey": "***"', content)


def run_wizard(closeclaw_bin: str, api_key: str, provider: str = "MiniMax") -> dict:
    """
    Run the config wizard interactively in a PTY.

    Returns dict with tmp_home, models_json, creds_json, minimax_model_ids,
    config_home.

    NOTE: The temp directory is created (not cleaned up automatically).
    Caller is responsible for removing it via shutil.rmtree when done.
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
        # ── Provider selection ─────────────────────────────────
        proc.expect("Select a provider", timeout=15)
        proc.sendline("0")  # MiniMax = index 0

        # ── API token ─────────────────────────────────────────
        proc.expect("API token", timeout=10)
        proc.sendline(api_key)

        # ── Model selection ───────────────────────────────────
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

        # ── Confirm ───────────────────────────────────────────
        proc.expect("Confirm?", timeout=10)
        proc.sendline("yes")

        # ── Wait for write ────────────────────────────────────
        proc.expect("Configuration written", timeout=15)

    except Exception as e:
        proc.terminate()
        raise RuntimeError(f"Wizard failed: {e}") from e
    finally:
        proc.close()

    # ── Verify output files ──────────────────────────────────
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

    minimax_models = models_data.get("providers", {}).get("minimax", {}).get("models", [])
    minimax_model_ids = {m["id"] for m in minimax_models}

    return {
        "tmp_home": tmp_home,
        "models_json": models_data,
        "creds_json": creds_data,
        "minimax_model_ids": minimax_model_ids,
        "config_home": config_home,
    }


# ─────────────────────────────────────────────────────────────────
# Step 2: Daemon lifecycle
# ─────────────────────────────────────────────────────────────────

class DaemonProcess:
    """
    Context manager for the closeclaw daemon process.

    Creates a temp directory as cwd (containing the config), and cleans
    up both the daemon process and the temp directory on exit.
    """

    def __init__(self, binary: Path, config_home: Path, log_prefix: str = "daemon"):
        self.binary = binary
        self.config_home = config_home  # Path to .closeclaw/
        self.log_prefix = log_prefix
        self.proc: subprocess.Popen | None = None
        self.log_file = None
        # The working dir is the parent of .closeclaw/ (the temp HOME)
        self._work_dir = str(config_home.parent)
        self._log_path = None

    def start(self) -> str:
        """Start daemon, return config_home path."""
        self.log_file = tempfile.NamedTemporaryFile(
            mode="w+", prefix=f"{self.log_prefix}-", suffix=".log", delete=False
        )
        self._log_path = self.log_file.name
        env = {
            **os.environ,
            "HOME": str(self._work_dir),
        }
        self.proc = subprocess.Popen(
            [str(self.binary), "run"],
            cwd=str(self._work_dir),
            env=env,
            stdout=self.log_file,
            stderr=subprocess.STDOUT,
        )
        # Wait for daemon to be ready
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
        if self.log_file:
            self.log_file.close()
        # Clean up temp working directory (，包含 .closeclaw/ 和日志文件)
        work_dir_str = str(self._work_dir)
        if work_dir_str.startswith(tempfile.gettempdir()):
            shutil.rmtree(self._work_dir, ignore_errors=True)

    def is_running(self) -> bool:
        if self.proc is None:
            return False
        return self.proc.poll() is None

    def logs(self) -> str:
        if self._log_path and os.path.exists(self._log_path):
            with open(self._log_path) as f:
                return f.read()
        return ""


# ─────────────────────────────────────────────────────────────────
# Step 3: CLI chat (single-shot TCP)
# ─────────────────────────────────────────────────────────────────

def run_chat(
    binary: Path,
    config_home: Path,
    message: str,
    addr: str = DEFAULT_CHAT_ADDR,
    agent_id: str = "guide",
) -> str:
    """
    Run `closeclaw chat --message ...` as a subprocess.
    Returns stdout (excluding INFO logs).
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
    # Filter out info-level log lines (ANSI codes + timestamp + INFO)
    lines = []
    for line in result.stdout.splitlines():
        if re.match(r"\d{4}-\d{2}-\d{2}T[\d:.]+Z\s+\[\d+m\s+INFO", line):
            continue
        lines.append(line)
    return "\n".join(lines).strip()


# ─────────────────────────────────────────────────────────────────
# Main
# ─────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(description="Full user flow: config wizard → chat")
    parser.add_argument(
        "--api-key",
        default=os.environ.get("MINIMAX_API_KEY", ""),
        help="MiniMax API key (default: MINIMAX_API_KEY env var)",
    )
    parser.add_argument(
        "--provider",
        default="MiniMax",
        help="Provider to configure (default: MiniMax)",
    )
    parser.add_argument(
        "--wizard-only",
        action="store_true",
        help="Only run the config wizard, skip daemon + chat",
    )
    parser.add_argument(
        "--chat-only",
        action="store_true",
        help="Skip wizard, assume config already exists at ~/.closeclaw/",
    )
    parser.add_argument(
        "--chat-addr",
        default=DEFAULT_CHAT_ADDR,
        help=f"Chat server address (default: {DEFAULT_CHAT_ADDR})",
    )
    parser.add_argument(
        "--agent-id",
        default="guide",
        help="Agent ID to chat with (default: guide)",
    )
    parser.add_argument(
        "--message",
        default="Hello, who are you?",
        help="Test message to send (default: 'Hello, who are you?')",
    )
    args = parser.parse_args()

    # ── Build binary ──────────────────────────────────────────────
    print(f"[1/3] Building binary: {BINARY}")
    build = subprocess.run(
        ["cargo", "build", "--bin", "closeclaw"],
        cwd=REPO,
        capture_output=True,
        text=True,
    )
    if build.returncode != 0:
        print("Build FAILED:")
        print(build.stderr[-2000:])
        sys.exit(1)
    print("[OK] Binary built")

    # ── Step 1: Config Wizard ──────────────────────────────────────
    if args.chat_only:
        config_home = Path.home() / ".closeclaw"
        if not config_home.exists():
            print("ERROR: --chat-only but ~/.closeclaw does not exist")
            sys.exit(1)
        print(f"[SKIP] Wizard (using existing config at {config_home})")
        wizard_result = None
    else:
        if not args.api_key:
            print("ERROR: --api-key required (or set MINIMAX_API_KEY env var)")
            sys.exit(1)
        print(f"[2/3] Running config wizard (provider={args.provider}, key=***)")
        wizard_result = run_wizard(BINARY, args.api_key, args.provider)
        config_home = wizard_result["config_home"]
        print(f"[OK] Config written to {config_home}")
        print(f"     Models: {sorted(wizard_result['minimax_model_ids'])}")

    if args.wizard_only:
        # Clean up temp dir ourselves
        if wizard_result:
            shutil.rmtree(wizard_result["tmp_home"], ignore_errors=True)
        print("[OK] Wizard-only mode, exiting")
        return

    # ── Step 2: Daemon ─────────────────────────────────────────────
    if wizard_result:
        daemon_config_home = wizard_result["config_home"]
    else:
        daemon_config_home = config_home

    print(f"[3/3] Starting daemon (config_dir={daemon_config_home})")
    daemon = DaemonProcess(BINARY, daemon_config_home)
    try:
        daemon.start()
        if not daemon.is_running():
            print("[FAIL] Daemon exited early. Logs:")
            print(daemon.logs()[-3000:])
            sys.exit(1)
        print("[OK] Daemon running")

        # ── Step 3: Chat ───────────────────────────────────────────
        print(f"Sending chat message: {args.message!r}")
        output = run_chat(
            BINARY,
            daemon_config_home,
            args.message,
            addr=args.chat_addr,
            agent_id=args.agent_id,
        )
        print("Chat output:")
        print(output)

    finally:
        print("Stopping daemon and cleaning up...")
        daemon.stop()
        print("[OK] Done")


if __name__ == "__main__":
    main()