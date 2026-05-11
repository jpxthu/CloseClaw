#!/usr/bin/env python3
"""
Full user flow test: config wizard → daemon → CLI chat.

Usage:
    python3 scripts/test_flow/full_user_flow.py [--api-key KEY]
        [--wizard-only] [--chat-only] [--message MSG]

Requires:
    pip install pexpect
"""

import argparse
import os
import shutil
import sys
from pathlib import Path

# Shared utilities
sys.path.insert(0, str(Path(__file__).parent))
from test_helpers import BINARY, build_binary, run_wizard, DaemonProcess, run_chat

DEFAULT_CHAT_ADDR = "127.0.0.1:18889"


def main():
    parser = argparse.ArgumentParser(
        description="Full user flow: config wizard → daemon → chat"
    )
    parser.add_argument(
        "--api-key",
        default=os.environ.get("MINIMAX_API_KEY", ""),
        help="MiniMax API key (default: MINIMAX_API_KEY env var)",
    )
    parser.add_argument(
        "--wizard-only",
        action="store_true",
        help="Only run the config wizard, skip daemon + chat",
    )
    parser.add_argument(
        "--chat-only",
        action="store_true",
        help="Skip wizard, use existing ~/.closeclaw/ config",
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
        help="Test message (default: 'Hello, who are you?')",
    )
    args = parser.parse_args()

    # ── Build ────────────────────────────────────────────────────
    print(f"[1/3] Building binary: {BINARY}")
    binary = build_binary()
    print("[OK] Binary built")

    # ── Wizard ───────────────────────────────────────────────────
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
        print("[2/3] Running config wizard (key=***)")
        wizard_result = run_wizard(binary, args.api_key)
        config_home = wizard_result["config_home"]
        print(f"[OK] Config written to {config_home}")
        print(f"     Models: {sorted(wizard_result['minimax_model_ids'])}")

    if args.wizard_only:
        if wizard_result:
            shutil.rmtree(wizard_result["tmp_home"], ignore_errors=True)
        print("[OK] Wizard-only mode, exiting")
        return

    # ── Daemon ───────────────────────────────────────────────────
    daemon_config_home = wizard_result["config_home"] if wizard_result else config_home
    print(f"[3/3] Starting daemon (config_dir={daemon_config_home})")
    daemon = DaemonProcess(binary, daemon_config_home)
    try:
        daemon.start()
        if not daemon.is_running():
            print("[FAIL] Daemon exited early. Logs:")
            print(daemon.logs()[-3000:])
            sys.exit(1)
        print("[OK] Daemon running")

        # ── Chat ─────────────────────────────────────────────────
        print(f"Sending: {args.message!r}")
        output = run_chat(
            binary,
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