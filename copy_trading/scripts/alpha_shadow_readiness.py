#!/usr/bin/env python3
"""Read-only Trade Service readiness gate for Alpha shadow operations."""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default=os.getenv("TRADE_SERVICE_URL", "http://127.0.0.1:18787"))
    parser.add_argument("--wallet-id", default=os.getenv("ALPHA_SHADOW_WALLET_ID"))
    args = parser.parse_args()
    headers = {"Accept": "application/json"}
    token = os.getenv("TRADE_SERVICE_TOKEN", "").strip()
    if token:
        headers["Authorization"] = f"Bearer {token}"
    request = urllib.request.Request(
        args.url.rstrip("/") + "/ready", headers=headers, method="GET"
    )
    try:
        with urllib.request.urlopen(request, timeout=3) as response:
            payload = json.load(response)
    except (OSError, urllib.error.HTTPError, json.JSONDecodeError) as error:
        print(json.dumps({"ready": False, "error": str(error)}, ensure_ascii=False))
        return 2

    wallets = payload.get("wallets") or []
    wallet_ok = bool(args.wallet_id) and any(
        wallet.get("wallet_id") == args.wallet_id for wallet in wallets
    )
    checks = {
        "ready": payload.get("ready") is True,
        "rpc_ok": payload.get("rpc_ok") is True,
        "has_blockhash": payload.get("has_blockhash") is True,
        "wallet_configured": wallet_ok,
    }
    result = {"mode": "alpha-shadow-readiness", "checks": checks, "payload": payload}
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0 if all(checks.values()) else 1


if __name__ == "__main__":
    sys.exit(main())
