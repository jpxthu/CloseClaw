#!/usr/bin/env python3
"""E2E test for `closeclaw config setup` wizard (MiniMax flow).

Run:  python3 scripts/test_flow/cli_config_wizard_test.py

This test verifies the full interactive wizard flow:
  1. Select MiniMax provider
  2. Enter API token
  3. Select models (knowledge-base fallback when API key is fake)
  4. Confirm
  5. Verify config files are written correctly

The test uses a temporary HOME directory so it does not interfere
with the user's real config at ~/.closeclaw/.

Shared utilities live in scripts/test_flow/test_helpers.py.
"""
from __future__ import annotations

import json
import os
import sys
from pathlib import Path

BINARY = os.environ.get(
    "CLOSE_CLAW_BINARY",
    Path(__file__).parent.parent / "target" / "debug" / "closeclaw",
)

# Import shared wizard runner from scripts/test_flow/test_helpers.py
sys.path.insert(0, str(Path(__file__).parent.parent / "scripts" / "test_flow"))
from test_helpers import run_wizard  # noqa: E402


def test_wizard_minimax():
    """End-to-end test: run the wizard with a fake key and verify output."""
    print(f"[TEST] binary: {BINARY}")
    print(f"[TEST] python: {sys.executable}")

    result = run_wizard(BINARY, api_key="abcd1234")

    # ── Credential file ─────────────────────────────────────────────────
    creds = result["creds_json"]
    assert creds.get("provider") == "minimax", f"unexpected provider: {creds.get('provider')}"
    assert creds.get("apiKey") == "abcd1234", \
        f"unexpected apiKey: {creds}"

    # ── Models file ────────────────────────────────────────────────────
    assert "minimax" in result["models_json"].get("providers", {}), \
        "minimax provider missing from models.json"

    model_ids = result["minimax_model_ids"]
    expected_models = {"MiniMax-M2", "MiniMax-M2.1", "MiniMax-M2.7", "MiniMax-M2.5"}
    missing = expected_models - model_ids
    assert not missing, f"expected models missing: {missing}"

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