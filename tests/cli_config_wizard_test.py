#!/usr/bin/env python3
"""E2E test for `closeclaw config setup` wizard (MiniMax flow).

Run:  python3 tests/cli_config_wizard_test.py
Or:   cargo test --test cli_config_wizard_test (via a thin Rust wrapper)

This test verifies the full interactive wizard flow:
  1. Select MiniMax provider
  2. Enter API token
  3. Select models (knowledge-base fallback when API key is fake)
  4. Confirm
  5. Verify config files are written correctly

The test uses a temporary HOME directory so it does not interfere
with the user's real config at ~/.closeclaw/.
"""
from __future__ import annotations

import glob
import json
import os
import re
import subprocess
import sys
import tempfile

import pexpect

BINARY = os.environ.get(
    "CLOSE_CLAW_BINARY",
    os.path.join(os.path.dirname(__file__), "..", "target", "debug", "closeclaw"),
)


def mask_api_key(content: str) -> str:
    """Mask api_key value in JSON for display."""
    return re.sub(r'"apiKey"\s*:\s*"[^"]+"', '"apiKey": "***"', content)


def run_wizard(closeclaw_bin: str, *, api_key: str = "fake-minimax-key-for-e2e-test") -> dict:
    """Run the wizard interactively in a PTY and return verification info."""
    with tempfile.TemporaryDirectory(prefix="closeclaw-wizard-test-") as tmp_home:
        env = {**os.environ, "HOME": tmp_home}

        proc = pexpect.spawn(
            closeclaw_bin,
            ["config", "setup"],
            encoding="utf-8",
            timeout=30,
            env=env,
            dimensions=(24, 80),
        )
        # proc.logfile_read = sys.stdout  # uncomment for debug

        try:
            # Step 1 — provider selection (MiniMax = index 0)
            proc.expect("Select a provider", timeout=15)
            proc.sendline("0")

            # Step 2 — API token
            proc.expect("Enter API token for MiniMax", timeout=10)
            proc.sendline(api_key)

            # Step 3 — model selection (may fallback to knowledge base)
            idx = proc.expect(
                [
                    "Your selection",  # API succeeded or fallback shows model list
                    "API fetch failed",
                    "timed out",
                    pexpect.TIMEOUT,
                    pexpect.EOF,
                ],
                timeout=30,
            )
            if idx in (1, 2):
                proc.expect("Your selection", timeout=10)
            elif idx >= 3:
                raise RuntimeError(f"Unexpected wizard state during model fetch (idx={idx})")

            proc.sendline("all")

            # Step 4 — confirm
            proc.expect("Confirm?", timeout=10)
            proc.sendline("yes")

            # Step 5 — wait for write confirmation
            proc.expect("Configuration written", timeout=15)

        finally:
            proc.close()

        # Verify written files
        models_path = os.path.join(tmp_home, ".closeclaw", "config", "models.json")
        creds_path = os.path.join(tmp_home, ".closeclaw", "config", "credentials", "minimax.json")

        if not os.path.exists(models_path):
            raise FileNotFoundError(f"models.json not written: {models_path}")
        if not os.path.exists(creds_path):
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
        }


def test_wizard_minimax():
    """End-to-end test: run the wizard with a fake key and verify output."""
    print(f"[TEST] binary: {BINARY}")
    print(f"[TEST] python: {sys.executable}")

    result = run_wizard(BINARY)

    # ── Credential file ─────────────────────────────────────────────────
    creds = result["creds_json"]
    assert creds.get("provider") == "minimax", f"unexpected provider: {creds.get('provider')}"
    assert creds.get("apiKey") == "fake-minimax-key-for-e2e-test", \
        f"unexpected apiKey: {mask_api_key(json.dumps(creds))}"

    # ── Models file ────────────────────────────────────────────────────
    assert "minimax" in result["models_json"].get("providers", {}), \
        "minimax provider missing from models.json"

    model_ids = result["minimax_model_ids"]
    # Should have at least the 4 known MiniMax models from knowledge base
    expected_models = {"MiniMax-M2", "MiniMax-M2.1", "MiniMax-M2.7", "MiniMax-M2.5"}
    missing = expected_models - model_ids
    assert not missing, f"expected models missing: {missing}"

    # All models should be enabled
    models = result["models_json"]["providers"]["minimax"]["models"]
    for m in models:
        assert m.get("enabled") is True, f"model {m['id']} should be enabled"

    print("\n[PASS] All assertions passed")
    print(f"  - credentials/minimax.json: provider={creds['provider']}, apiKey=***")
    print(f"  - models.json: minimax models = {sorted(model_ids)}")
    return True


if __name__ == "__main__":
    test_wizard_minimax()
    print("\n[OK] cli_config_wizard_test passed")