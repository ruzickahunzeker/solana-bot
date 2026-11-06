#!/usr/bin/env python3
"""Verify primary submission stays closed during SDK simulation migration."""

from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
values = {}
for line in (ROOT / ".env").read_text().splitlines():
    if "=" in line and not line.lstrip().startswith("#"):
        key, value = line.split("=", 1)
        values[key.strip()] = value.strip().strip('"').strip("'")


def require(key: str, expected: str) -> None:
    if values.get(key) != expected:
        raise SystemExit(f"{key} must be {expected!r} for simulate-only operation")


require("TRADE_SERVICE_URL", "http://127.0.0.1:18788")
require("TRADE_SUBMISSION_ENABLED", "false")
if values.get("TRADE_ISOLATED_SDK_SIMULATION_ENABLED", "false") not in ("true", "false"):
    raise SystemExit("TRADE_ISOLATED_SDK_SIMULATION_ENABLED must be true or false")
require("DRY_RUN_DEFAULT", "true")
require("TRADE_ROUTE_DECISION_URL", "http://127.0.0.1:18887")
if not values.get("TRADE_ROUTE_DECISION_TOKEN"):
    raise SystemExit("TRADE_ROUTE_DECISION_TOKEN must be set for isolated SDK decisions")
print("simulate-only Bot configuration passed")
