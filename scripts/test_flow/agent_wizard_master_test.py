#!/usr/bin/env python3
"""E2E test for master Agent creation via `closeclaw config setup` wizard.

需求: docs/requirements/agent.md §F5
运行: python3 scripts/test_flow/agent_wizard_master_test.py

Step 1.4 §F5: Verify that the setup wizard creates the master Agent
with full tool/skill permissions and registers it in agents.json.

This test uses a temporary HOME directory and pexpect to drive the
interactive wizard, then asserts on the written config files.

Shared utilities live in scripts/test_flow/test_helpers.py.
"""
from __future__ import annotations

import json
import os
import shutil
import sys
import tempfile
from pathlib import Path

BINARY = os.environ.get(
    "CLOSE_CLAW_BINARY",
    Path(__file__).parent.parent.parent / "target" / "debug" / "closeclaw",
)

sys.path.insert(0, str(Path(__file__).parent))
from test_helpers import run_wizard  # noqa: E402

import pexpect


def _run_wizard_twice(binary: str | Path, api_key: str = "test1234") -> dict:
    """Run the config setup wizard twice in the same temp HOME.

    Returns dict with:
        tmp_home: str
        config_home: Path — .closeclaw/ directory
        master_config: dict — parsed master/config.json
        agents_list: list[str] — parsed agents.json agents list
    """
    tmp_home = tempfile.mkdtemp(prefix="closeclaw-wizard-master-")
    env = {**os.environ, "HOME": tmp_home}

    def _drive_wizard():
        proc = pexpect.spawn(
            str(binary),
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

            # Confirm (yes/no)
            proc.expect("Confirm\\?", timeout=10)
            proc.sendline("yes")

            # Write config now? (Y/n)
            proc.expect("Write config now\\?", timeout=10)
            proc.sendline("yes")

            # Wait for write
            proc.expect("Configuration written", timeout=15)

            # Wait for process to finish writing master agent files
            proc.expect(pexpect.EOF, timeout=15)

        except Exception as e:
            proc.terminate()
            raise RuntimeError(f"Wizard failed: {e}") from e
        finally:
            proc.close()

    # ── First run ───────────────────────────────────────────────────────
    _drive_wizard()

    config_home = Path(tmp_home) / ".closeclaw"
    master_config_path = config_home / "agents" / "master" / "config.json"
    agents_json_path = config_home / "config" / "agents.json"

    if not master_config_path.exists():
        raise FileNotFoundError(f"master config.json not written: {master_config_path}")
    if not agents_json_path.exists():
        raise FileNotFoundError(f"agents.json not written: {agents_json_path}")

    with open(master_config_path) as f:
        master_config_first = json.load(f)
    with open(agents_json_path) as f:
        agents_json_first = json.load(f)

    # ── Second run (idempotency) ────────────────────────────────────────
    _drive_wizard()

    with open(master_config_path) as f:
        master_config_second = json.load(f)
    with open(agents_json_path) as f:
        agents_json_second = json.load(f)

    return {
        "tmp_home": tmp_home,
        "config_home": config_home,
        "master_config_first": master_config_first,
        "master_config_second": master_config_second,
        "agents_list_first": agents_json_first.get("agents", []),
        "agents_list_second": agents_json_second.get("agents", []),
    }


def test_wizard_creates_master_agent():
    """§F5: Wizard creates master Agent with full permissions."""
    print(f"[TEST] binary: {BINARY}")
    print(f"[TEST] python: {sys.executable}")

    result = _run_wizard_twice(BINARY)
    tmp_home = result["tmp_home"]

    try:
        mc = result["master_config_first"]
        agents = result["agents_list_first"]

        # ── Assertion 1: master/config.json exists with id=="master" ────
        assert mc.get("id") == "master", f"expected id=='master', got: {mc.get('id')}"

        # ── Assertion 2: tools contains "*" (all tools allowed) ─────────
        assert "*" in mc.get("tools", []), \
            f"expected tools to contain '*', got: {mc.get('tools')}"

        # ── Assertion 3: skills contains "*" (all skills allowed) ───────
        assert "*" in mc.get("skills", []), \
            f"expected skills to contain '*', got: {mc.get('skills')}"

        # ── Assertion 4: agents.json contains "master" ──────────────────
        assert "master" in agents, \
            f"expected 'master' in agents.json, got: {agents}"

        # ── Assertion 5: Idempotency — second run does not change config ─
        mc2 = result["master_config_second"]
        agents2 = result["agents_list_second"]
        assert mc == mc2, (
            "master/config.json changed after second wizard run!\n"
            f"  first:  {json.dumps(mc, indent=2)}\n"
            f"  second: {json.dumps(mc2, indent=2)}"
        )
        assert agents == agents2, (
            "agents.json changed after second wizard run!\n"
            f"  first:  {agents}\n"
            f"  second: {agents2}"
        )

        print("\n[PASS] §F5 — all assertions passed")
        print(f"  - agents/master/config.json: id={mc['id']}, tools={mc['tools']}, skills={mc['skills']}")
        print(f"  - agents.json agents: {agents}")
        print(f"  - Idempotent: config unchanged after re-run")

    finally:
        shutil.rmtree(tmp_home, ignore_errors=True)


if __name__ == "__main__":
    test_wizard_creates_master_agent()
    print("\n[OK] agent_wizard_master_test passed")
