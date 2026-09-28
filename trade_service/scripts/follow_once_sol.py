#!/usr/bin/env python3
import argparse
import json
import os
import signal
import subprocess
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path

import orjson
import zmq

ROOT = Path(__file__).resolve().parents[1]
BIN = "./target/release/pump-trade-service"
FULL_BIN = str((ROOT / "target/release/pump-trade-service").resolve())
API = "http://127.0.0.1:18787"
OWNER = "E9kibXXJQrn5GAboDzPNjwdyy8AEi9HvUf8Jfnxq9vRF"
WSOL = "So11111111111111111111111111111111111111112"
SOL = "So11111111111111111111111111111111111111111"
ENABLES = [
    "ENABLE_TEMPORAL",
    "ENABLE_FLASHBLOCK",
    "ENABLE_BLOCKRAZOR",
    "ENABLE_JITO",
    "ENABLE_ASTRALANE",
    "ENABLE_SPEEDLANDING",
]


def emit(obj):
    print(json.dumps(obj, ensure_ascii=False), flush=True)


def load_env():
    env = {}
    for line in (ROOT / ".env").read_text().splitlines():
        line = line.strip()
        if line and not line.startswith("#") and "=" in line:
            key, value = line.split("=", 1)
            env[key] = value.strip().strip('"').strip("'")
    return env


ENV = load_env()
TOKEN = ENV["LOCAL_TRADE_API_TOKEN"]
RPC = ENV["RPC_URL"]


def post_json(url, payload, headers=None, timeout=20):
    request = urllib.request.Request(
        url,
        data=json.dumps(payload).encode(),
        headers={"Content-Type": "application/json", **(headers or {})},
    )
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return json.loads(response.read().decode())


def get_json(url, headers=None, timeout=5):
    request = urllib.request.Request(url, headers=headers or {})
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return json.loads(response.read().decode())


def rpc(method, params, timeout=10):
    return post_json(RPC, {"jsonrpc": "2.0", "id": 1, "method": method, "params": params}, timeout=timeout)


def sol_lamports():
    return int(rpc("getBalance", [OWNER, {"commitment": "processed"}])["result"]["value"])


def token_raw(mint):
    result = rpc(
        "getTokenAccountsByOwner",
        [OWNER, {"mint": mint}, {"encoding": "jsonParsed", "commitment": "processed"}],
    )
    return sum(
        int(item["account"]["data"]["parsed"]["info"]["tokenAmount"]["amount"])
        for item in result.get("result", {}).get("value", [])
    )


def signature_status(signature):
    if not signature:
        return None
    values = rpc("getSignatureStatuses", [[signature], {"searchTransactionHistory": True}]).get(
        "result", {}
    ).get("value") or []
    return values[0] if values else None


def wait_signature_and_balance(signature, mint, want_zero=False, deadline=45):
    start = time.perf_counter()
    visible_ms = confirmed_ms = balance_event_ms = None
    last_balance = None
    last_status = None
    while time.perf_counter() - start < deadline:
        now = time.perf_counter()
        try:
            status = signature_status(signature)
        except Exception:
            status = None
        if status:
            last_status = status
        if status and visible_ms is None:
            visible_ms = (now - start) * 1000
        if (
            status
            and status.get("confirmationStatus") in ("processed", "confirmed", "finalized")
            and confirmed_ms is None
        ):
            confirmed_ms = (now - start) * 1000
        try:
            balance = token_raw(mint)
        except Exception:
            balance = None
        if balance is not None:
            last_balance = balance
            if want_zero and balance == 0 and balance_event_ms is None:
                balance_event_ms = (now - start) * 1000
            if not want_zero and balance > 0 and balance_event_ms is None:
                balance_event_ms = (now - start) * 1000
        if confirmed_ms is not None and balance_event_ms is not None:
            break
        time.sleep(0.2)
    return {
        "visible_ms": visible_ms,
        "confirmed_ms": confirmed_ms,
        "balance_event_ms": balance_event_ms,
        "last_balance_raw": last_balance,
        "confirmation": (last_status or {}).get("confirmationStatus") if last_status else None,
    }


def service_pids():
    output = subprocess.run(["ps", "-eo", "pid=,args="], capture_output=True, text=True).stdout
    pids = []
    for line in output.splitlines():
        line = line.strip()
        if not line:
            continue
        pid_text, args = line.split(None, 1)
        first = args.split()[0] if args.split() else ""
        if first in (FULL_BIN, BIN) or first.endswith("/pump-trade-service"):
            pids.append(int(pid_text))
    return pids


def kill_existing_service():
    for pid in service_pids():
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    start = time.time()
    while service_pids() and time.time() - start < 5:
        time.sleep(0.2)


def start_service_temporal():
    kill_existing_service()
    env = os.environ.copy()
    env.update(ENV)
    env["RUST_LOG"] = "info"
    env["MEV_USE_BACKUP_RPCS"] = "false"
    for key in ENABLES:
        env[key] = "false"
    env["ENABLE_TEMPORAL"] = "true"
    log = open("/tmp/follow-once-sol-temporal.log", "w")
    return subprocess.Popen([BIN], cwd=str(ROOT), env=env, stdout=log, stderr=subprocess.STDOUT, text=True)


def restore_default_service():
    kill_existing_service()
    env = os.environ.copy()
    env.update(ENV)
    env["RUST_LOG"] = "info"
    log = open("/tmp/follow-once-sol-restore.log", "w")
    return subprocess.Popen([BIN], cwd=str(ROOT), env=env, stdout=log, stderr=subprocess.STDOUT, text=True)


def wait_ready(proc, expected=None, timeout=120):
    start = time.time()
    last = None
    while time.time() - start < timeout:
        if proc.poll() is not None:
            raise RuntimeError(f"service exited {proc.returncode}")
        try:
            ready = get_json(f"{API}/ready", {"Authorization": "Bearer " + TOKEN}, 5)
            last = ready
            lanes = ready.get("active_swqos_lanes") or []
            if ready.get("ready") and (expected is None or expected in [lane.lower() for lane in lanes]):
                return ready
        except Exception:
            pass
        time.sleep(1)
    raise TimeoutError(f"ready timeout {last}")


def text_field(event, *names):
    for name in names:
        value = event.get(name)
        if isinstance(value, str) and value:
            return value
    return None


def involved_wallets(event):
    wallets = set()
    traders = event.get("tradersInvolved") or event.get("traders_involved")
    if isinstance(traders, dict):
        wallets.update(str(key) for key in traders.keys())
    elif isinstance(traders, list):
        wallets.update(str(item) for item in traders)
    for name in ("txSigner", "signer", "wallet", "user"):
        value = event.get(name)
        if isinstance(value, str) and value:
            wallets.add(value)
    return wallets


def pool_alias(pool):
    if not pool:
        return None
    normalized = pool.strip().lower().replace("_", "-").replace(" ", "-")
    return {
        "pump": "pump",
        "pumpfun": "pump",
        "pump-fun": "pump",
        "pump-amm": "pump-amm",
        "pumpswap": "pump-amm",
        "pump-swap": "pump-amm",
        "bonk": "bonk",
        "meteora": "meteora-damm-v2",
        "meteora-damm-v2": "meteora-damm-v2",
        "raydium-amm-v4": "raydium-amm-v4",
        "raydium-cpmm": "raydium-cpmm",
    }.get(normalized)


def subscribe_once(args):
    context = zmq.Context.instance()
    sub = context.socket(zmq.SUB)
    sub.setsockopt(zmq.RCVHWM, 10000)
    sub.connect(args.zmq_endpoint)
    sub.setsockopt_string(zmq.SUBSCRIBE, "")
    poller = zmq.Poller()
    poller.register(sub, zmq.POLLIN)
    start = time.time()
    scanned = 0
    skipped = 0
    emit({"event": "subscribed_sol_only", "wallet": args.wallet, "max_wait_s": args.max_wait_s})
    while time.time() - start < args.max_wait_s:
        if sub not in dict(poller.poll(1000)):
            continue
        parts = sub.recv_multipart()
        if len(parts) != 1:
            continue
        scanned += 1
        try:
            event = orjson.loads(parts[0])
        except Exception:
            continue
        if not isinstance(event, dict):
            continue
        if (text_field(event, "action", "txType") or "").lower() != "buy":
            continue
        if args.wallet not in involved_wallets(event):
            continue
        seen_ms = int(time.time() * 1000)
        mint = text_field(event, "mint")
        pool = pool_alias(text_field(event, "pool", "dex", "amm"))
        pool_id = text_field(event, "poolId", "pool_id", "ammId", "amm_id")
        quote_mint = text_field(event, "quoteMint", "quote_mint") or WSOL
        candidate = {
            "seen_ms": seen_ms,
            "mint": mint,
            "pool": pool,
            "pool_id": pool_id,
            "quote_mint": quote_mint,
            "source_signature": text_field(event, "signature"),
            "scanned": scanned,
            "skipped": skipped,
            "event_timestamp_ms": event.get("timestamp"),
            "marketCapQuote": event.get("marketCapQuote"),
            "price": event.get("price"),
        }
        if quote_mint not in (WSOL, SOL):
            skipped += 1
            emit({"event": "target_buy_skip", "reason": "non_sol_quote", "candidate": candidate})
            continue
        if not mint or not pool:
            skipped += 1
            emit({"event": "target_buy_skip", "reason": "missing mint/pool", "candidate": candidate})
            continue
        if pool in ("pump-amm", "meteora-damm-v2", "raydium-amm-v4", "raydium-cpmm") and not pool_id:
            skipped += 1
            emit({"event": "target_buy_skip", "reason": "missing pool_id", "candidate": candidate})
            continue
        return candidate
    return None


def trade(candidate, side, args):
    payload = {
        "side": side,
        "pool": candidate["pool"],
        "mint": candidate["mint"],
        "pool_id": candidate.get("pool_id"),
        "quote_mint": candidate.get("quote_mint"),
        "amount_sol": args.buy_sol if side == "buy" else None,
        "sell_percent": 100 if side == "sell" else None,
        "slippage_percent": args.slippage_percent,
        "tip_sol": args.tip_sol,
        "execution_method": "sdk",
        "use_mev": True,
        "mev_provider": args.provider,
        "simulate": False,
        "source": "follow-wallet",
        "source_signature": candidate.get("source_signature"),
        "idempotency_key": f"follow-once-sol-{side}-{uuid.uuid4()}",
    }
    payload = {key: value for key, value in payload.items() if value is not None}
    start = time.perf_counter()
    wall_ms = int(time.time() * 1000)
    try:
        response = post_json(f"{API}/v1/trades", payload, {"Authorization": "Bearer " + TOKEN}, 85)
        error = None
    except urllib.error.HTTPError as exc:
        response = None
        error = f"HTTP {exc.code}: {exc.read().decode(errors='replace')[:300]}"
    except Exception as exc:
        response = None
        error = repr(exc)
    return {
        "http_ms": (time.perf_counter() - start) * 1000,
        "from_event_ms": wall_ms - candidate["seen_ms"],
        "resp": response,
        "error": error,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--wallet", required=True)
    parser.add_argument("--buy-sol", type=float, default=0.002)
    parser.add_argument("--tip-sol", type=float, default=0.0001)
    parser.add_argument("--provider", default="temporal")
    parser.add_argument("--slippage-percent", type=float, default=20)
    parser.add_argument("--max-wait-s", type=int, default=1800)
    parser.add_argument("--zmq-endpoint", default="tcp://127.0.0.1:8939")
    args = parser.parse_args()

    emit({
        "event": "start",
        "wallet": args.wallet,
        "initial_sol_lamports": sol_lamports(),
        "buy_sol": args.buy_sol,
        "tip_lamports": int(args.tip_sol * 1_000_000_000),
    })
    proc = None
    try:
        proc = start_service_temporal()
        ready = wait_ready(proc, args.provider)
        emit({"event": "service_ready", "lanes": ready.get("active_swqos_lanes")})
        candidate = subscribe_once(args)
        if not candidate:
            emit({"event": "timeout", "message": "no SOL quote target buy during wait"})
            return
        emit({"event": "target_buy_tradeable", "candidate": candidate})
        buy = trade(candidate, "buy", args)
        buy_sig = (buy.get("resp") or {}).get("signature")
        buy_wait = (
            wait_signature_and_balance(buy_sig, candidate["mint"], False, 45)
            if buy_sig
            else {"last_balance_raw": token_raw(candidate["mint"])}
        )
        emit({
            "event": "follow_buy_done",
            "http_ms": buy["http_ms"],
            "from_event_to_request_ms": buy["from_event_ms"],
            "status": (buy.get("resp") or {}).get("status"),
            "signature": buy_sig,
            "error": buy.get("error") or (buy.get("resp") or {}).get("error"),
            "wait": buy_wait,
        })
        if (buy_wait.get("last_balance_raw") or 0) > 0:
            sell = trade(candidate, "sell", args)
            sell_sig = (sell.get("resp") or {}).get("signature")
            sell_wait = (
                wait_signature_and_balance(sell_sig, candidate["mint"], True, 45)
                if sell_sig
                else {"last_balance_raw": token_raw(candidate["mint"])}
            )
            emit({
                "event": "follow_sell_done",
                "http_ms": sell["http_ms"],
                "from_event_to_sell_request_ms": sell["from_event_ms"],
                "status": (sell.get("resp") or {}).get("status"),
                "signature": sell_sig,
                "error": sell.get("error") or (sell.get("resp") or {}).get("error"),
                "wait": sell_wait,
                "final_raw": token_raw(candidate["mint"]),
            })
        else:
            emit({"event": "sell_skipped", "final_raw": token_raw(candidate["mint"])})
    finally:
        emit({"event": "restore_default"})
        restored = restore_default_service()
        try:
            ready = wait_ready(restored, None, 120)
            restore = {"ready": ready.get("ready"), "lanes": ready.get("active_swqos_lanes")}
        except Exception as exc:
            restore = {"error": repr(exc)}
        emit({"event": "done", "restore": restore, "final_sol_lamports": sol_lamports()})


if __name__ == "__main__":
    main()
