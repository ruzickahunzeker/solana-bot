#!/usr/bin/env python3
import asyncio
import hashlib
import os
import sys
import time
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
SCHEMA_VERSION = 1
DEFAULT_OBSERVATION_SPOOL = Path(__file__).resolve().parent / "data" / "pumpapi_observation.jsonl"


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


def now_ms() -> int:
    return time.time_ns() // 1_000_000


def canonical_hash(event: dict) -> str:
    payload = orjson.dumps(event, option=orjson.OPT_SORT_KEYS)
    return hashlib.sha256(payload).hexdigest()


def event_id(event: dict, raw_hash: str | None = None) -> str:
    """Return a stable ID; unsigned events remain observation-only."""
    signature = text_field(event, "signature", "txSignature", "tx_signature")
    action = text_field(event, "action", "txType", "tx_type") or "unknown"
    mint = text_field(event, "mint") or "unknown"
    pool_id = text_field(event, "poolId", "pool_id", "ammId", "amm_id") or "unknown"
    if signature:
        return f"{signature}:{action}:{mint}:{pool_id}"
    return f"sha256:{raw_hash or canonical_hash(event)}"


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
        "schema_version": SCHEMA_VERSION,
        "event_id": event_id(event),
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


def normalize_observation(event: dict, raw_hash: str, received_at_ms: int):
    """Normalize an event for research/replay without requiring a trade route."""
    event_pool = text_field(event, "pool", "dex", "amm")
    mint = text_field(event, "mint")
    action = text_field(event, "action", "txType", "tx_type")
    pool = trade_service_pool(event_pool, event, mint) if event_pool else None
    quote_mint = normalized_quote_mint(event_pool, event) if event_pool else None
    route_status = (
        "executable"
        if pool and quote_mint
        else ("observable-only" if event_pool else "unknown")
    )
    signature = text_field(event, "signature", "txSignature", "tx_signature")
    return {
        "schema_version": SCHEMA_VERSION,
        "event_id": event_id(event, raw_hash),
        "event_time_ms": event.get("timestamp"),
        "received_at_ms": received_at_ms,
        "source": "pumpapi",
        "action": action,
        "event_pool": event_pool,
        "trade_service_pool": pool,
        "route_status": route_status,
        "mint": mint,
        "quote_mint": quote_mint,
        "pool_id": text_field(event, "poolId", "pool_id", "ammId", "amm_id"),
        "tx_signer": text_field(event, "txSigner", "signer"),
        "signature": signature,
        "token_amount": event.get("tokenAmount"),
        "quote_amount": event.get("quoteAmount"),
        "decimals": event.get("decimals"),
        "token_program": text_field(event, "tokenProgram", "token_program"),
        "sol_in_pool": event.get("solInPool"),
        "quote_in_pool": event.get("quoteInPool"),
        "tokens_in_pool": event.get("tokensInPool"),
        "platform": text_field(event, "platform"),
        "programs_used": event.get("programsUsed"),
        "address_lookup_tables": event.get("addressLookupTables"),
        "raw_hash": f"sha256:{raw_hash}",
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


def open_spool_file():
    path = observation_spool_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    return path.open("ab")


def observation_spool_path() -> Path:
    configured = os.getenv("PUMPAPI_OBSERVATION_SPOOL_FILE")
    return Path(configured) if configured else DEFAULT_OBSERVATION_SPOOL


async def stream_to_zmq(sock: zmq.Socket) -> None:
    stream_url = os.getenv("PUMPAPI_STREAM_URL", DEFAULT_STREAM_URL)
    reconnect_delay = env_int("PUMPAPI_RECONNECT_SECONDS", 1)
    capture_limit = env_int("PUMPAPI_CAPTURE_LIMIT", 0)
    raw_capture = open_capture_file("PUMPAPI_CAPTURE_FILE")
    normalized_capture = open_capture_file("PUMPAPI_NORMALIZED_CAPTURE_FILE")
    observation_spool = open_spool_file()
    spool_path = observation_spool_path()
    spool_max_bytes = env_int("PUMPAPI_OBSERVATION_SPOOL_MAX_BYTES", 256 * 1024 * 1024)
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

                    received_at_ms = now_ms()
                    observation = None
                    raw_hash = None
                    try:
                        event = orjson.loads(raw)
                    except orjson.JSONDecodeError:
                        event = None
                    if isinstance(event, dict):
                        raw_hash = canonical_hash(event)
                        observation = normalize_observation(event, raw_hash, received_at_ms)
                        observation_bytes = orjson.dumps(observation)
                        observation_spool.write(observation_bytes)
                        observation_spool.write(b"\n")
                        observation_spool.flush()
                        if spool_max_bytes and observation_spool.tell() >= spool_max_bytes:
                            observation_spool.close()
                            rotated = spool_path.with_name(f"{spool_path.name}.1")
                            try:
                                rotated.unlink()
                            except FileNotFoundError:
                                pass
                            spool_path.replace(rotated)
                            observation_spool = open_spool_file()
                        sock.send_multipart([b"observation", observation_bytes])

                    # Optional normalized channel: easier to feed trade_service tests.
                    if publish_normalized:
                        if isinstance(event, dict):
                            item = normalize_for_trade_service(event)
                            if item:
                                item["received_at_ms"] = received_at_ms
                                item["raw_hash"] = f"sha256:{raw_hash}"
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
