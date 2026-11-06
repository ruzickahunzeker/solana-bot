#!/usr/bin/env python3
"""Run one SDK 6 PumpSwap buy simulation against a disposable service and databases."""

import json
import os
import atexit
from pathlib import Path
import secrets
import socket
import sqlite3
import subprocess
import tempfile
import time
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


ROOT = Path(__file__).resolve().parents[1]
BINARY = Path(os.environ.get("SDK6_ISOLATED_BINARY", ""))
SOURCE_WALLETS = Path(os.environ.get("SDK6_SOURCE_WALLETS", "/tmp/candidate-staging-data/wallets.sqlite"))
TERMINAL = {"simulated", "confirmed", "failed", "rejected", "unknown"}


def call(base, token, path, body=None, timeout=30):
    data = None if body is None else json.dumps(body).encode()
    headers = {"authorization": f"Bearer {token}", "content-type": "application/json"}
    try:
        with urlopen(Request(base + path, data=data, headers=headers), timeout=timeout) as response:
            return response.status, json.load(response)
    except HTTPError as error:
        return error.code, json.loads(error.read())


def start(binary, work, env, base):
    log = (work / "service.log").open("a")
    process = subprocess.Popen([str(binary)], cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT)
    for _ in range(240):
        if process.poll() is not None:
            log.close()
            raise RuntimeError("isolated SDK 6 service exited before health; inspect private service.log")
        try:
            with urlopen(base + "/health", timeout=1) as response:
                if json.load(response).get("ok") is True:
                    return process, log
        except (OSError, URLError):
            time.sleep(0.25)
    process.terminate()
    process.wait(timeout=5)
    log.close()
    raise TimeoutError("isolated SDK 6 service did not become healthy")


def stop(process, log):
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)
    log.close()


def terminal_order(base, token, request):
    code, first = call(base, token, "/v1/trades", request, timeout=90)
    if code not in (200, 202):
        raise RuntimeError(f"trade request returned HTTP {code}")
    order_id = first["request_id"]
    for _ in range(80):
        _, record = call(base, token, f"/v1/trades/{order_id}")
        if record.get("status") in TERMINAL:
            break
        time.sleep(0.5)
    else:
        raise TimeoutError("SDK 6 simulation did not reach a terminal state")
    duplicate_code, duplicate = call(base, token, "/v1/trades", request)
    if duplicate_code != 200 or duplicate.get("request_id") != order_id:
        raise AssertionError("same idempotency key did not return original order")
    return order_id, record


def main():
    if not BINARY.is_file() or not SOURCE_WALLETS.is_file() or not os.environ.get("RPC_URL"):
        raise RuntimeError("set SDK6_ISOLATED_BINARY, RPC_URL and SDK6_SOURCE_WALLETS")
    work = Path(tempfile.mkdtemp(prefix="sdk6-single-pool-sim-"))
    work.chmod(0o700)
    with sqlite3.connect(f"file:{SOURCE_WALLETS}?mode=ro", uri=True) as source:
        with sqlite3.connect(work / "wallets.sqlite") as target:
            source.backup(target)
    with sqlite3.connect(work / "wallets.sqlite") as wallet_db:
        wallet_db.execute("DELETE FROM wallets WHERE wallet_id != '2TE'")
        wallet_db.execute("UPDATE wallets SET is_default = 1 WHERE wallet_id = '2TE'")
    def remove_wallet_copy():
        for suffix in ("", "-wal", "-shm"):
            (work / f"wallets.sqlite{suffix}").unlink(missing_ok=True)

    atexit.register(remove_wallet_copy)
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    base = f"http://127.0.0.1:{port}"
    token = secrets.token_urlsafe(32)
    env = dict(
        os.environ,
        TRADE_HTTP_ADDR=f"127.0.0.1:{port}",
        LOCAL_TRADE_API_TOKEN=token,
        WALLET_STORAGE_MODE="database",
        DEFAULT_WALLET_ID="2TE",
        WALLET_DATABASE_URL=f"sqlite://{work / 'wallets.sqlite'}",
        WALLETS_CONFIG_ENCRYPTED="",
        WALLETS_CONFIG_ENC="",
        WALLETS_CONFIG="",
        TRADE_PERSISTENCE_MODE="durable",
        TRADE_DATABASE_URL=f"sqlite://{work / 'trades.sqlite'}",
        CONVERSION_DATABASE_URL=f"sqlite://{work / 'conversions.sqlite'}",
        CONVERSION_LIVE_ENABLED="false",
        CONVERSION_QUOTES_ENABLED="false",
        ENABLE_SIMULATE="true",
        WAIT_TX_CONFIRMED="false",
        ENABLE_PUMPAPI_EXECUTOR="false",
        ENABLE_MEV_PROTECTION="false",
        REQUIRE_MEV_SUBMIT="false",
        MEV_USE_BACKUP_RPCS="false",
        ENABLE_JITO="false",
        ENABLE_TEMPORAL="false",
        ENABLE_FLASHBLOCK="false",
        ENABLE_BLOCKRAZOR="false",
        ENABLE_ASTRALANE="false",
        ENABLE_SPEEDLANDING="false",
        BACKUP_RPC_URL="",
        BACKUP_RPC_URLS="",
        RPC_BACKUP_URLS="",
        RUST_LOG="warn",
    )
    request = {
        "wallet_id": "2TE",
        "side": "buy",
        "pool": "pump-amm",
        "mint": "pumpCmXqMfrsAkQ5r49WcJnRayYRqmXz6ae8H7H9Dfn",
        "pool_id": "539m4mVWt6iduB6W8rDGPMarzNCMesuqY5eUTiiYHAgR",
        "quote_mint": "So11111111111111111111111111111111111111112",
        "amount_lamports": 1_000_000,
        "slippage_bps": 100,
        "tip_sol": 0.0,
        "execution_method": "sdk",
        "use_mev": False,
        "simulate": True,
        "source": "isolated-sdk6-verification",
        "idempotency_key": "sdk6-sim-" + secrets.token_hex(8),
    }
    sell_request = {
        "wallet_id": "2TE",
        "side": "sell",
        "pool": "raydium-cpmm",
        "mint": "So11111111111111111111111111111111111111112",
        "pool_id": "fAjTnZ9QqJkUmrr8cXutkYhpVge2qqtSZNt9qKn7YC2",
        "quote_mint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        "settlement_asset": "quote",
        "sell_percent": 1,
        "slippage_bps": 100,
        "tip_sol": 0.0,
        "execution_method": "sdk",
        "use_mev": False,
        "simulate": True,
        "source": "isolated-sdk6-verification",
        "idempotency_key": "sdk6-sell-sim-" + secrets.token_hex(8),
    }
    process, log = start(BINARY, work, env, base)
    try:
        ready_code, ready = call(base, token, "/ready")
        if ready_code != 200 or not ready.get("ready"):
            raise RuntimeError("isolated SDK 6 RPC or blockhash is not ready")
        order_id, record = terminal_order(base, token, request)
        sell_order_id, sell_record = terminal_order(base, token, sell_request)
    finally:
        stop(process, log)
    process, log = start(BINARY, work, env, base)
    try:
        for checked_id, checked_record in ((order_id, record), (sell_order_id, sell_record)):
            _, reloaded = call(base, token, f"/v1/trades/{checked_id}")
            if reloaded.get("request_id") != checked_id or reloaded.get("status") != checked_record.get("status"):
                raise AssertionError("durable order changed across restart")
    finally:
        stop(process, log)
    recovery_db = work / "recovery.sqlite"
    with sqlite3.connect(work / "trades.sqlite") as source:
        with sqlite3.connect(recovery_db) as target:
            source.backup(target)
    with sqlite3.connect(recovery_db) as interrupted:
        interrupted.execute(
            "UPDATE trades SET status='submitting', signature=NULL WHERE request_id=?",
            (sell_order_id,),
        )
    recovery_env = dict(env, TRADE_DATABASE_URL=f"sqlite://{recovery_db}")
    process, log = start(BINARY, work, recovery_env, base)
    try:
        _, recovered = call(base, token, f"/v1/trades/{sell_order_id}")
        replay_code, replay = call(base, token, "/v1/trades", sell_request)
        with sqlite3.connect(recovery_db) as persisted:
            recovery_rows = persisted.execute("SELECT COUNT(*) FROM trades").fetchone()[0]
        if (recovered.get("status") != "unknown"
                or replay_code != 200
                or replay.get("request_id") != sell_order_id
                or replay.get("status") != "unknown"
                or recovery_rows != 2):
            raise AssertionError("interrupted submitting order was replayed or not recovered")
    finally:
        stop(process, log)
    result = {
        "status": record.get("status"),
        "signature": record.get("signature"),
        "sell_status": sell_record.get("status"),
        "sell_signature": sell_record.get("signature"),
        "same_idempotency_order": True,
        "restart_readback": True,
        "interrupted_submitting_recovered_unknown": True,
        "interrupted_order_replayed": False,
        "recovery_trade_rows": recovery_rows,
        "work_dir": str(work),
    }
    (work / "RESULT.json").write_text(json.dumps(result, indent=2) + "\n")
    remove_wallet_copy()
    print(json.dumps(result, sort_keys=True))
    if any(item.get("status") != "simulated" or item.get("signature") is not None
           for item in (record, sell_record)):
        raise AssertionError("SDK 6 did not return two signature-free simulated orders")


if __name__ == "__main__":
    main()
