#!/usr/bin/env python3
import asyncio
import os
import sys
from pathlib import Path

import orjson
import websockets
import zmq


DEFAULT_STREAM_URL = "wss://stream.pumpapi.io"
DEFAULT_BIND = "tcp://127.0.0.1:8939"
WSOL_MINT = "So11111111111111111111111111111111111111112"
LEGACY_SOL_QUOTE_POOLS = {"pump", "pumpfun", "pump-fun"}
STONKFUN_PLATFORM_CONFIGS = {
    "4E876qZTE9FJMrBzgVtBrSrzz2TLivB5Y5QXPjB4gZL7",
    "6BwHHDg3u1854jC8PDLXvR4spTcLNaoBxLJNGC4nTESt",
}


def stonkfun_route_file():
    configured = os.getenv("STONKFUN_ROUTE_FILE")
    if configured:
        return Path(configured)
    return Path(__file__).resolve().parent / "data" / "stonkfun_mints.txt"


def load_stonkfun_mints():
    try:
        return {
            line.strip()
            for line in stonkfun_route_file().read_text().splitlines()
            if line.strip()
        }
    except FileNotFoundError:
        return set()


STONKFUN_MINTS = load_stonkfun_mints()


def remember_stonkfun_mint(mint):
    if not mint or mint in STONKFUN_MINTS:
        return
    STONKFUN_MINTS.add(mint)
    path = stonkfun_route_file()
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="ascii") as output:
        output.write(f"{mint}\n")


def env_int(name: str, default: int) -> int:
    value = os.getenv(name)
    if not value:
        return default
    return int(value)


def bind_pub_socket() -> zmq.Socket:
    ctx = zmq.Context.instance()
    sock = ctx.socket(zmq.PUB)
    sock.setsockopt(zmq.SNDHWM, env_int("ZMQ_SNDHWM", 10000))

    bind_values = os.getenv("ZMQ_BIND", DEFAULT_BIND)
    for endpoint in [value.strip() for value in bind_values.split(",") if value.strip()]:
        sock.bind(endpoint)
        print(f"zmq pub bind {endpoint}", file=sys.stderr, flush=True)
    return sock


def normalize_for_trade_service(event: dict):
    event_pool = text_field(event, "pool", "dex", "amm")
    mint = text_field(event, "mint")
    if not event_pool or not mint:
        return None

    pool = trade_service_pool(event_pool, event, mint)
    if not pool:
        return None

    quote_mint = normalized_quote_mint(event_pool, event)
    action = text_field(event, "action", "txType", "tx_type")

    return {
        "trade_service_pool": pool,
        "event_pool": event_pool,
        "mint": mint,
        "pool_id": text_field(event, "poolId", "pool_id", "ammId", "amm_id"),
        "quote_mint": quote_mint,
        "platform": text_field(event, "platform"),
        "launchpad_config": text_field(event, "launchpadConfig", "launchpad_config"),
        "pool_created_by": text_field(event, "poolCreatedBy", "pool_created_by"),
        "token_program": text_field(event, "tokenProgram", "token_program"),
        "tx_type": action,
        "action": action,
        "signature": text_field(event, "signature"),
        "tx_signer": text_field(event, "txSigner", "signer"),
        "timestamp": event.get("timestamp"),
        "quote_in_pool": event.get("quoteInPool"),
        "sol_in_pool": event.get("solInPool"),
        "tokens_in_pool": event.get("tokensInPool"),
        "quote_amount": event.get("quoteAmount"),
        "token_amount": event.get("tokenAmount"),
        "decimals": event.get("decimals"),
        "programs_used": event.get("programsUsed"),
        "address_lookup_tables": event.get("addressLookupTables"),
    }


def trade_service_pool(pool: str, event=None, mint=None):
    normalized = pool.strip().lower().replace("_", "-").replace(" ", "-")
    event = event or {}
    launchpad_config = text_field(event, "launchpadConfig", "launchpad_config")
    platform = text_field(event, "platform")
    if normalized in {"raydium-launchpad", "raydium-launchlab"}:
        if (platform and platform.strip().lower() == "stonkfun") or (
            launchpad_config in STONKFUN_PLATFORM_CONFIGS
        ):
            remember_stonkfun_mint(mint)
            return "stonkfun"
        return None
    if normalized in {"raydium-cpmm", "raydium-cp"} and mint in STONKFUN_MINTS:
        return "stonkfun-swap"
    aliases = {
        "pump": "pump",
        "pumpfun": "pump",
        "pump-fun": "pump",
        "pump-amm": "pump-amm",
        "pumpswap": "pump-amm",
        "pump-swap": "pump-amm",
        "bonk": "bonk",
        "meteora": "meteora-damm-v2",
        "meteora-damm-v2": "meteora-damm-v2",
        "meteora-dammv2": "meteora-damm-v2",
        "meteora-dbc": "meteora-dbc",
        "meteora-dynamic-bonding-curve": "meteora-dbc",
        "dynamic-bonding-curve": "meteora-dbc",
        "dbc": "meteora-dbc",
        "raydium-amm-v4": "raydium-amm-v4",
        "raydium-ammv4": "raydium-amm-v4",
        "raydium-v4": "raydium-amm-v4",
        "raydium-cpmm": "raydium-cpmm",
        "raydium-cp": "raydium-cpmm",
    }
    return aliases.get(normalized)


def normalized_quote_mint(pool: str, event: dict):
    quote_mint = text_field(event, "quoteMint", "quote_mint")
    if quote_mint:
        return quote_mint
    normalized = pool.strip().lower().replace("_", "-").replace(" ", "-")
    if normalized in LEGACY_SOL_QUOTE_POOLS:
        return WSOL_MINT
    return None


def text_field(event: dict, *names):
    for name in names:
        value = event.get(name)
        if isinstance(value, str) and value:
            return value
    return None


def open_capture_file(env_name: str):
    path = os.getenv(env_name)
    if not path:
        return None
    capture_path = Path(path)
    capture_path.parent.mkdir(parents=True, exist_ok=True)
    return capture_path.open("ab")


async def stream_to_zmq(sock: zmq.Socket) -> None:
    stream_url = os.getenv("PUMPAPI_STREAM_URL", DEFAULT_STREAM_URL)
    reconnect_delay = env_int("PUMPAPI_RECONNECT_SECONDS", 1)
    capture_limit = env_int("PUMPAPI_CAPTURE_LIMIT", 0)
    raw_capture = open_capture_file("PUMPAPI_CAPTURE_FILE")
    normalized_capture = open_capture_file("PUMPAPI_NORMALIZED_CAPTURE_FILE")
    publish_normalized = os.getenv("ZMQ_PUBLISH_NORMALIZED", "true").lower() not in {
        "0",
        "false",
        "no",
        "off",
    }

    sent = 0
    while True:
        try:
            print(f"pumpapi connect {stream_url}", file=sys.stderr, flush=True)
            async with websockets.connect(stream_url) as ws:
                print("pumpapi connected", file=sys.stderr, flush=True)
                async for msg in ws:
                    if isinstance(msg, str):
                        raw = msg.encode()
                    else:
                        raw = bytes(msg)

                    # Raw channel: compatible with PumpAPI official ZMQ example.
                    sock.send(raw)
                    if raw_capture:
                        raw_capture.write(raw)
                        raw_capture.write(b"\n")
                        raw_capture.flush()

                    # Optional normalized channel: easier to feed trade_service tests.
                    if publish_normalized:
                        try:
                            event = orjson.loads(raw)
                        except orjson.JSONDecodeError:
                            event = None
                        if isinstance(event, dict):
                            item = normalize_for_trade_service(event)
                            if item:
                                normalized = orjson.dumps(item)
                                sock.send_multipart([b"normalized", normalized])
                                if normalized_capture:
                                    normalized_capture.write(normalized)
                                    normalized_capture.write(b"\n")
                                    normalized_capture.flush()

                    sent += 1
                    if capture_limit and sent >= capture_limit:
                        print(f"captured {sent}; exiting", file=sys.stderr, flush=True)
                        return
        except Exception as exc:
            print(f"pumpapi stream error: {exc}", file=sys.stderr, flush=True)
            await asyncio.sleep(reconnect_delay)


def main() -> int:
    sock = bind_pub_socket()
    asyncio.run(stream_to_zmq(sock))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
