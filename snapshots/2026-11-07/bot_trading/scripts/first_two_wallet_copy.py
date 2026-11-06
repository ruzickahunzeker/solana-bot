#!/usr/bin/env python3
"""Follow wallet A when it buys within a mint's first two PumpAPI trades.

The script consumes the local PumpAPI ZMQ bridge and submits through the
existing LAN trade-service SSH tunnel. It is dry-run by default; pass --live
only after readiness and wallet binding have been verified.
"""

from __future__ import annotations

import argparse
import os
import sys
import threading
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.request import ProxyHandler, Request, build_opener

import orjson
import zmq


WSOL_MINT = "So11111111111111111111111111111111111111112"


def load_env() -> dict[str, str]:
    values: dict[str, str] = {}
    path = Path(__file__).resolve().parent.parent / ".env"
    if not path.exists():
        return values
    for line in path.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        values[key.strip()] = value.strip().strip('"').strip("'")
    return values


def first_text(event: dict, *names: str) -> str | None:
    for name in names:
        value = event.get(name)
        if isinstance(value, str) and value.strip():
            return value.strip()
    return None


def buy_traders(event: dict) -> set[str]:
    traders: set[str] = set()
    breakdown = event.get("breakdown")
    has_breakdown = isinstance(breakdown, list)
    if has_breakdown:
        for item in breakdown:
            if not isinstance(item, dict):
                continue
            action = str(item.get("action", "")).lower()
            trader = item.get("trader")
            if action == "buy" and isinstance(trader, str) and trader:
                traders.add(trader)
    action = str(event.get("action") or event.get("tx_type") or "").lower()
    if not has_breakdown and action == "buy" or (not has_breakdown and action == "create" and event.get("initialBuy")):
        for name in ("txSigner", "tx_signer", "signer"):
            trader = event.get(name)
            if isinstance(trader, str) and trader:
                traders.add(trader)
    traders_involved = event.get("tradersInvolved")
    if not has_breakdown and isinstance(traders_involved, dict):
        traders.update(str(item) for item in traders_involved if item)
    return traders


def is_buy_transaction(event: dict) -> bool:
    action = str(event.get("action") or event.get("tx_type") or "").lower()
    breakdown = event.get("breakdown")
    if isinstance(breakdown, list) and any(
        isinstance(item, dict) and str(item.get("action", "")).lower() == "buy"
        for item in breakdown
    ):
        return True
    if action == "buy":
        return True
    if action == "create":
        if event.get("initialBuy"):
            return True
        breakdown = event.get("breakdown")
        return isinstance(breakdown, list) and any(
            isinstance(item, dict) and str(item.get("action", "")).lower() == "buy"
            for item in breakdown
        )
    return False


def decode_frames(frames: list[bytes]) -> dict | None:
    if len(frames) != 1:
        return None
    try:
        value = orjson.loads(frames[0])
    except orjson.JSONDecodeError:
        return None
    return value if isinstance(value, dict) else None


class FirstTwoTracker:
    def __init__(self, wallet_a: str):
        self.wallet_a = wallet_a
        self.states: dict[str, dict] = {}

    def observe(self, event: dict) -> dict | None:
        mint = first_text(event, "mint")
        signature = first_text(event, "signature")
        action = str(event.get("action") or event.get("tx_type") or "").lower()
        if not mint or not signature or (action != "create" and not is_buy_transaction(event)):
            return None
        traders = buy_traders(event)
        state = self.states.setdefault(mint, {"count": 0, "signatures": {}, "created": False, "triggered": False})
        if not state["created"] and action != "create":
            return {"type": "wallet_buy_unranked", "mint": mint, "signature": signature, "wallet_a_hit": False}
        pool_id = first_text(event, "poolId", "pool_id", "ammId", "amm_id")
        if signature in state["signatures"] and state["signatures"][signature] != pool_id:
            return {"type": "ranked_buy", "mint": mint, "signature": signature, "order": state["count"], "wallet_a_hit": self.wallet_a in traders, "buyers": sorted(traders)}
        if signature in state["signatures"] and state["signatures"][signature] == pool_id:
            return None
        if state["count"] >= 2 or state["triggered"]:
            return None
            return None
        state["signatures"][signature] = pool_id
        state["created"] = state["created"] or action == "create"
        state["count"] += 1
        order = state["count"]
        hit = self.wallet_a in traders
        if hit:
            state["triggered"] = True
        return {"type": "ranked_buy", "mint": mint, "signature": signature, "order": order, "wallet_a_hit": hit, "buyers": sorted(traders)}


class TradeClient:
    def __init__(self, env: dict[str, str], live: bool, wallet_id: str | None):
        self.base = env.get("TRADE_SERVICE_URL", "http://127.0.0.1:18788").rstrip("/")
        self.token = env.get("TRADE_SERVICE_TOKEN", "")
        self.live = live
        self.wallet_id = wallet_id
        self.amount_sol = float(env.get("DEFAULT_BUY_AMOUNT_SOL", "0.01"))
        self.slippage = float(env.get("DEFAULT_SLIPPAGE_BPS", "1500")) / 100.0
        self.tip_sol = float(env.get("DEFAULT_TIP_SOL", "0.0001"))
        self.opener = build_opener(ProxyHandler({}))

    def call(self, path: str, body: dict | None = None) -> tuple[int, dict]:
        headers = {"Accept": "application/json"}
        if self.token:
            headers["Authorization"] = "Bearer " + self.token
        data = None
        if body is not None:
            headers["Content-Type"] = "application/json"
            data = orjson.dumps(body)
        request = Request(self.base + path, data=data, headers=headers)
        try:
            with self.opener.open(request, timeout=5) as response:
                raw = response.read()
                return response.status, orjson.loads(raw or b"{}")
        except HTTPError as error:
            return error.code, {}
        except (URLError, TimeoutError, OSError, orjson.JSONDecodeError):
            return 0, {}

    def resolve_wallet(self) -> str:
        if self.wallet_id:
            return self.wallet_id
        status, ready = self.call("/ready")
        if status != 200 or not ready.get("ready"):
            raise RuntimeError("trade-service is not ready")
        wallet_id = ready.get("default_wallet_id")
        if not isinstance(wallet_id, str) or not wallet_id:
            raise RuntimeError("trade-service returned no default wallet")
        self.wallet_id = wallet_id
        return wallet_id

    def submit(self, event: dict, wallet_a: str) -> None:
        mint = first_text(event, "mint")
        signature = first_text(event, "signature")
        pool = first_text(event, "pool", "event_pool", "dex", "amm")
        if not mint or not signature or not pool:
            print(orjson.dumps({"status": "blocked", "reason": "missing mint/signature/pool"}).decode(), flush=True)
            return
        wallet_id = self.resolve_wallet()
        request = {
            "wallet_id": wallet_id,
            "execution_method": "sdk",
            "side": "buy",
            "pool": pool,
            "mint": mint,
            "pool_id": first_text(event, "poolId", "pool_id", "ammId", "amm_id"),
            "quote_mint": first_text(event, "quoteMint", "quote_mint") or WSOL_MINT,
            "amount_sol": self.amount_sol,
            "slippage_percent": self.slippage,
            "tip_sol": self.tip_sol,
            "use_mev": False,
            "mev_provider": None,
            "simulate": not self.live,
            "source": "pumpapi-first-two-wallet-copy",
            "source_signature": signature,
            "idempotency_key": f"first-two:{wallet_a}:{mint}:buy",
        }
        status, result = self.call("/v1/trades", request)
        print(
            orjson.dumps(
                {
                    "status": "submitted" if status in (200, 201, 202) else "failed",
                    "http": status,
                    "mint": mint,
                    "source_signature": signature,
                    "request_id": result.get("request_id"),
                    "trade_status": result.get("status"),
                    "simulate": not self.live,
                },
            ).decode(),
            flush=True,
        )


def main() -> int:
    env = load_env()
    parser = argparse.ArgumentParser()
    parser.add_argument("--wallet-a", default=os.getenv("FOLLOW_WALLET_A") or env.get("FOLLOW_WALLET_A"))
    parser.add_argument(
        "--endpoint",
        default=os.getenv("PUMPAPI_ZMQ_ENDPOINT") or env.get("PUMPAPI_ZMQ_ENDPOINT", "tcp://127.0.0.1:8939"),
    )
    parser.add_argument("--wallet-id", default=os.getenv("TRADE_SERVICE_WALLET_ID") or env.get("TRADE_SERVICE_WALLET_ID"))
    parser.add_argument("--amount-sol", type=float)
    parser.add_argument("--live", action="store_true", help="submit live trades; default is simulate")
    parser.add_argument("--max-inflight", type=int, default=16)
    args = parser.parse_args()
    if not args.wallet_a:
        parser.error("--wallet-a or FOLLOW_WALLET_A is required")
    if args.max_inflight < 1:
        parser.error("--max-inflight must be greater than zero")

    if args.amount_sol is not None:
        env["DEFAULT_BUY_AMOUNT_SOL"] = str(args.amount_sol)
    if args.live and env.get("TRADE_SUBMISSION_ENABLED", "false").lower() != "true":
        parser.error("TRADE_SUBMISSION_ENABLED must be true for --live")

    client = TradeClient(env, args.live, args.wallet_id)
    if args.live:
        client.resolve_wallet()

    context = zmq.Context.instance()
    socket = context.socket(zmq.SUB)
    socket.setsockopt(zmq.SUBSCRIBE, b"")
    socket.connect(args.endpoint)
    print(orjson.dumps({"ready": True, "endpoint": args.endpoint, "live": args.live}).decode(), flush=True)

    states: dict[str, dict] = {}
    lock = threading.Lock()
    executor = ThreadPoolExecutor(max_workers=args.max_inflight)

    try:
        while True:
            raw = socket.recv()
            try:
                event = orjson.loads(raw)
            except orjson.JSONDecodeError:
                continue
            if not isinstance(event, dict) or not is_buy_transaction(event):
                continue
            mint = first_text(event, "mint")
            signature = first_text(event, "signature")
            if not mint or not signature:
                continue
            traders = buy_traders(event)
            with lock:
                state = states.setdefault(mint, {"count": 0, "signatures": set(), "triggered": False})
                if state["triggered"] or signature in state["signatures"] or state["count"] >= 2:
                    continue
                state["signatures"].add(signature)
                state["count"] += 1
                order = state["count"]
                hit = args.wallet_a in traders
                if hit:
                    state["triggered"] = True
            print(orjson.dumps({"mint": mint, "order": order, "wallet_a_hit": hit}).decode(), flush=True)
            if hit:
                executor.submit(client.submit, event, args.wallet_a)
    except KeyboardInterrupt:
        return 0
    finally:
        socket.close(0)
        context.term()
        executor.shutdown(wait=False, cancel_futures=True)


if __name__ == "__main__":
    sys.exit(main())
