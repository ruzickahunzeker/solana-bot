#!/usr/bin/env python3
import argparse
import hashlib
import os
import sqlite3
import sys
import time
from pathlib import Path

import orjson
import zmq


DEFAULT_ENDPOINT = "tcp://127.0.0.1:8939"
DEFAULT_DB = "/root/http/sol-bot/pumpapi_zmq_bridge/data/wallet_watch.sqlite"
DEFAULT_WALLETS = (
    "CCCCQCrL6zVjnDeucDzcxJgxAs5ahNmrhw1CDexPhqrd,"
    "nya666pQkP3PzWxi7JngU3rRMHuc7zbLK8c8wxQ4qpT"
)
DEFAULT_QUOTE_MINT = "So11111111111111111111111111111111111111112"
DEFAULT_HIGH_TRACK_SECONDS = 600


def env_int(name: str, default: int) -> int:
    value = os.getenv(name)
    if not value:
        return default
    return int(value)


def env_float(name: str, default: float) -> float:
    value = os.getenv(name)
    if not value:
        return default
    return float(value)


def parse_csv(value: str) -> list[str]:
    return [item.strip() for item in value.split(",") if item.strip()]


def now_ms() -> int:
    return int(time.time() * 1000)


def text_field(event: dict, *names: str) -> str | None:
    for name in names:
        value = event.get(name)
        if isinstance(value, str) and value:
            return value
    return None


def int_field(event: dict, *names: str) -> int | None:
    for name in names:
        value = event.get(name)
        if value is None:
            continue
        try:
            return int(value)
        except (TypeError, ValueError):
            continue
    return None


def float_field(event: dict, *names: str) -> float | None:
    for name in names:
        value = event.get(name)
        if value is None:
            continue
        try:
            return float(value)
        except (TypeError, ValueError):
            continue
    return None


def open_db(path: str) -> sqlite3.Connection:
    db_path = Path(path)
    db_path.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(str(db_path))
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA busy_timeout=5000")
    migrate(conn)
    return conn


def migrate(conn: sqlite3.Connection) -> None:
    conn.executescript(
        """
        CREATE TABLE IF NOT EXISTS wallet_buy_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_key TEXT NOT NULL UNIQUE,
            wallet TEXT NOT NULL,
            mint TEXT NOT NULL,
            pool TEXT,
            pool_id TEXT,
            quote_mint TEXT,
            signature TEXT NOT NULL,
            tx_signer TEXT,
            token_amount REAL,
            quote_amount REAL,
            price REAL,
            market_cap_quote REAL,
            block INTEGER,
            event_timestamp_ms INTEGER,
            observed_at_ms INTEGER NOT NULL,
            raw_json TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_wallet_buy_events_mint
            ON wallet_buy_events (mint, event_timestamp_ms);
        CREATE INDEX IF NOT EXISTS idx_wallet_buy_events_wallet
            ON wallet_buy_events (wallet, event_timestamp_ms);
        CREATE INDEX IF NOT EXISTS idx_wallet_buy_events_signature
            ON wallet_buy_events (signature);

        CREATE TABLE IF NOT EXISTS mint_wallet_matches (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            match_key TEXT NOT NULL UNIQUE,
            mint TEXT NOT NULL,
            first_wallet TEXT NOT NULL,
            second_wallet TEXT NOT NULL,
            first_event_id INTEGER NOT NULL,
            second_event_id INTEGER NOT NULL,
            first_signature TEXT NOT NULL,
            second_signature TEXT NOT NULL,
            first_pool TEXT,
            second_pool TEXT,
            first_pool_id TEXT,
            second_pool_id TEXT,
            first_timestamp_ms INTEGER,
            second_timestamp_ms INTEGER,
            time_delta_ms INTEGER,
            matched_at_ms INTEGER NOT NULL,
            FOREIGN KEY(first_event_id) REFERENCES wallet_buy_events(id),
            FOREIGN KEY(second_event_id) REFERENCES wallet_buy_events(id)
        );

        CREATE INDEX IF NOT EXISTS idx_mint_wallet_matches_mint
            ON mint_wallet_matches (mint, matched_at_ms);

        CREATE TABLE IF NOT EXISTS mint_10m_highs (
            mint TEXT PRIMARY KEY,
            first_match_id INTEGER NOT NULL,
            first_matched_at_ms INTEGER NOT NULL,
            tracking_started_at_ms INTEGER NOT NULL,
            tracking_until_ms INTEGER NOT NULL,
            tracking_done INTEGER NOT NULL DEFAULT 0,
            event_count INTEGER NOT NULL DEFAULT 0,
            high_price REAL,
            high_price_signature TEXT,
            high_price_event_timestamp_ms INTEGER,
            high_price_observed_at_ms INTEGER,
            high_market_cap_quote REAL,
            high_market_cap_quote_signature TEXT,
            high_market_cap_quote_event_timestamp_ms INTEGER,
            high_market_cap_quote_observed_at_ms INTEGER,
            high_market_cap_sol REAL,
            high_market_cap_sol_signature TEXT,
            high_market_cap_sol_event_timestamp_ms INTEGER,
            high_market_cap_sol_observed_at_ms INTEGER,
            last_signature TEXT,
            last_event_timestamp_ms INTEGER,
            last_observed_at_ms INTEGER,
            updated_at_ms INTEGER NOT NULL,
            FOREIGN KEY(first_match_id) REFERENCES mint_wallet_matches(id)
        );

        CREATE INDEX IF NOT EXISTS idx_mint_10m_highs_until
            ON mint_10m_highs (tracking_done, tracking_until_ms);
        """
    )
    conn.commit()


def involved_wallets(event: dict) -> set[str]:
    wallets: set[str] = set()
    traders = event.get("tradersInvolved") or event.get("traders_involved")
    if isinstance(traders, dict):
        wallets.update(str(key) for key in traders.keys() if key)
    elif isinstance(traders, list):
        wallets.update(str(item) for item in traders if item)

    if not wallets:
        signer = text_field(event, "txSigner", "signer", "wallet", "user")
        if signer:
            wallets.add(signer)
    return wallets


def event_key(wallet: str, event: dict) -> str:
    parts = {
        "wallet": wallet,
        "signature": text_field(event, "signature") or "",
        "action": text_field(event, "action", "txType") or "",
        "mint": text_field(event, "mint") or "",
        "pool": text_field(event, "pool", "dex", "amm") or "",
        "pool_id": text_field(event, "poolId", "pool_id", "ammId", "amm_id") or "",
        "quote_mint": text_field(event, "quoteMint", "quote_mint") or DEFAULT_QUOTE_MINT,
        "timestamp": int_field(event, "timestamp") or 0,
        "token_amount": float_field(event, "tokenAmount", "baseAmount", "initialBuy") or 0,
        "quote_amount": float_field(event, "quoteAmount", "solAmount", "amount") or 0,
    }
    raw = orjson.dumps(parts, option=orjson.OPT_SORT_KEYS)
    return hashlib.sha256(raw).hexdigest()


def normalize_action(event: dict) -> str:
    return (text_field(event, "action", "txType") or "").strip().lower()


def insert_buy_event(
    conn: sqlite3.Connection,
    wallet: str,
    event: dict,
    observed_at_ms: int,
) -> tuple[int | None, bool]:
    mint = text_field(event, "mint")
    signature = text_field(event, "signature")
    if not mint or not signature:
        return None, False

    key = event_key(wallet, event)
    raw_json = orjson.dumps(event, option=orjson.OPT_SORT_KEYS).decode()
    params = {
        "event_key": key,
        "wallet": wallet,
        "mint": mint,
        "pool": text_field(event, "pool", "dex", "amm"),
        "pool_id": text_field(event, "poolId", "pool_id", "ammId", "amm_id"),
        "quote_mint": text_field(event, "quoteMint", "quote_mint") or DEFAULT_QUOTE_MINT,
        "signature": signature,
        "tx_signer": text_field(event, "txSigner", "signer"),
        "token_amount": float_field(event, "tokenAmount", "baseAmount", "initialBuy"),
        "quote_amount": float_field(event, "quoteAmount", "solAmount", "amount"),
        "price": float_field(event, "price"),
        "market_cap_quote": float_field(event, "marketCapQuote", "market_cap_quote"),
        "block": int_field(event, "block", "slot"),
        "event_timestamp_ms": int_field(event, "timestamp"),
        "observed_at_ms": observed_at_ms,
        "raw_json": raw_json,
    }
    before = conn.total_changes
    conn.execute(
        """
        INSERT OR IGNORE INTO wallet_buy_events (
            event_key, wallet, mint, pool, pool_id, quote_mint, signature, tx_signer,
            token_amount, quote_amount, price, market_cap_quote, block,
            event_timestamp_ms, observed_at_ms, raw_json
        ) VALUES (
            :event_key, :wallet, :mint, :pool, :pool_id, :quote_mint, :signature, :tx_signer,
            :token_amount, :quote_amount, :price, :market_cap_quote, :block,
            :event_timestamp_ms, :observed_at_ms, :raw_json
        )
        """,
        params,
    )
    inserted = conn.total_changes > before
    row = conn.execute(
        "SELECT id FROM wallet_buy_events WHERE event_key = ?",
        (key,),
    ).fetchone()
    return (int(row["id"]) if row else None), inserted


def create_matches(
    conn: sqlite3.Connection,
    event_id: int,
    match_window_ms: int,
) -> list[sqlite3.Row]:
    current = conn.execute(
        "SELECT * FROM wallet_buy_events WHERE id = ?",
        (event_id,),
    ).fetchone()
    if not current:
        return []

    event_ts = current["event_timestamp_ms"] or current["observed_at_ms"]
    candidates = conn.execute(
        """
        SELECT * FROM wallet_buy_events
        WHERE mint = ?
          AND wallet != ?
          AND ABS(COALESCE(event_timestamp_ms, observed_at_ms) - ?) <= ?
        ORDER BY COALESCE(event_timestamp_ms, observed_at_ms) DESC
        """,
        (current["mint"], current["wallet"], event_ts, match_window_ms),
    ).fetchall()

    matches = []
    for other in candidates:
        first, second = sorted([current, other], key=lambda row: int(row["id"]))
        match_key = f"{first['id']}:{second['id']}"
        first_ts = first["event_timestamp_ms"] or first["observed_at_ms"]
        second_ts = second["event_timestamp_ms"] or second["observed_at_ms"]
        delta_ms = abs(int(first_ts) - int(second_ts))
        before = conn.total_changes
        conn.execute(
            """
            INSERT OR IGNORE INTO mint_wallet_matches (
                match_key, mint, first_wallet, second_wallet,
                first_event_id, second_event_id,
                first_signature, second_signature,
                first_pool, second_pool, first_pool_id, second_pool_id,
                first_timestamp_ms, second_timestamp_ms, time_delta_ms, matched_at_ms
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                match_key,
                current["mint"],
                first["wallet"],
                second["wallet"],
                first["id"],
                second["id"],
                first["signature"],
                second["signature"],
                first["pool"],
                second["pool"],
                first["pool_id"],
                second["pool_id"],
                first["event_timestamp_ms"],
                second["event_timestamp_ms"],
                delta_ms,
                now_ms(),
            ),
        )
        if conn.total_changes > before:
            match = conn.execute(
                "SELECT * FROM mint_wallet_matches WHERE match_key = ?",
                (match_key,),
            ).fetchone()
            if match:
                matches.append(match)
    return matches


def seed_recent_high_trackers(conn: sqlite3.Connection, high_track_ms: int) -> int:
    cutoff_ms = now_ms() - high_track_ms
    rows = conn.execute(
        """
        SELECT *
        FROM mint_wallet_matches
        WHERE matched_at_ms >= ?
        ORDER BY matched_at_ms ASC
        """,
        (cutoff_ms,),
    ).fetchall()
    created = 0
    for row in rows:
        if ensure_high_tracker(conn, row, high_track_ms):
            created += 1
    conn.commit()
    return created


def ensure_high_tracker(
    conn: sqlite3.Connection,
    match: sqlite3.Row,
    high_track_ms: int,
) -> bool:
    start_ms = int(match["matched_at_ms"])
    before = conn.total_changes
    conn.execute(
        """
        INSERT OR IGNORE INTO mint_10m_highs (
            mint, first_match_id, first_matched_at_ms,
            tracking_started_at_ms, tracking_until_ms,
            tracking_done, event_count, updated_at_ms
        ) VALUES (?, ?, ?, ?, ?, 0, 0, ?)
        """,
        (
            match["mint"],
            match["id"],
            match["matched_at_ms"],
            start_ms,
            start_ms + high_track_ms,
            now_ms(),
        ),
    )
    return conn.total_changes > before


def finalize_expired_high_trackers(conn: sqlite3.Connection, observed_at_ms: int) -> int:
    before = conn.total_changes
    conn.execute(
        """
        UPDATE mint_10m_highs
        SET tracking_done = 1, updated_at_ms = ?
        WHERE tracking_done = 0
          AND tracking_until_ms < ?
        """,
        (observed_at_ms, observed_at_ms),
    )
    return conn.total_changes - before


def update_active_highs_for_event(
    conn: sqlite3.Connection,
    event: dict,
    observed_at_ms: int,
) -> int:
    mint = text_field(event, "mint")
    if not mint:
        return 0

    tracker = conn.execute(
        """
        SELECT *
        FROM mint_10m_highs
        WHERE mint = ?
          AND tracking_done = 0
          AND tracking_started_at_ms <= ?
          AND tracking_until_ms >= ?
        """,
        (mint, observed_at_ms, observed_at_ms),
    ).fetchone()
    if not tracker:
        return 0

    event_timestamp_ms = int_field(event, "timestamp")
    signature = text_field(event, "signature")
    price = float_field(event, "price")
    market_cap_quote = float_field(event, "marketCapQuote", "market_cap_quote")
    market_cap_sol = float_field(event, "marketCapSol", "market_cap_sol")

    updates = {
        "event_count": int(tracker["event_count"]) + 1,
        "last_signature": signature,
        "last_event_timestamp_ms": event_timestamp_ms,
        "last_observed_at_ms": observed_at_ms,
        "updated_at_ms": observed_at_ms,
    }

    if is_new_high(price, tracker["high_price"]):
        updates.update(
            {
                "high_price": price,
                "high_price_signature": signature,
                "high_price_event_timestamp_ms": event_timestamp_ms,
                "high_price_observed_at_ms": observed_at_ms,
            }
        )
    if is_new_high(market_cap_quote, tracker["high_market_cap_quote"]):
        updates.update(
            {
                "high_market_cap_quote": market_cap_quote,
                "high_market_cap_quote_signature": signature,
                "high_market_cap_quote_event_timestamp_ms": event_timestamp_ms,
                "high_market_cap_quote_observed_at_ms": observed_at_ms,
            }
        )
    if is_new_high(market_cap_sol, tracker["high_market_cap_sol"]):
        updates.update(
            {
                "high_market_cap_sol": market_cap_sol,
                "high_market_cap_sol_signature": signature,
                "high_market_cap_sol_event_timestamp_ms": event_timestamp_ms,
                "high_market_cap_sol_observed_at_ms": observed_at_ms,
            }
        )

    assignments = ", ".join(f"{name} = :{name}" for name in updates)
    updates["mint"] = mint
    before = conn.total_changes
    conn.execute(
        f"UPDATE mint_10m_highs SET {assignments} WHERE mint = :mint",
        updates,
    )
    changed = conn.total_changes - before
    if changed and any(
        name in updates
        for name in ("high_price", "high_market_cap_quote", "high_market_cap_sol")
    ):
        print(
            f"mint_10m_high mint={mint} "
            f"price={updates.get('high_price', tracker['high_price'])} "
            f"market_cap_quote={updates.get('high_market_cap_quote', tracker['high_market_cap_quote'])} "
            f"market_cap_sol={updates.get('high_market_cap_sol', tracker['high_market_cap_sol'])} "
            f"signature={signature}",
            file=sys.stderr,
            flush=True,
        )
    return changed


def is_new_high(value: float | None, current: float | None) -> bool:
    if value is None:
        return False
    if current is None:
        return True
    return value > float(current)


def process_event(
    conn: sqlite3.Connection,
    event: dict,
    watch_wallets: set[str],
    watch_actions: set[str],
    match_window_ms: int,
    high_track_ms: int,
) -> tuple[int, int]:
    observed_at_ms = now_ms()
    high_changes = finalize_expired_high_trackers(conn, observed_at_ms)
    high_changes += update_active_highs_for_event(conn, event, observed_at_ms)

    if normalize_action(event) not in watch_actions:
        if high_changes:
            conn.commit()
        return 0, 0
    mint = text_field(event, "mint")
    if not mint:
        if high_changes:
            conn.commit()
        return 0, 0

    hit_wallets = sorted(involved_wallets(event) & watch_wallets)
    if not hit_wallets:
        if high_changes:
            conn.commit()
        return 0, 0

    inserted_count = 0
    match_count = 0
    for wallet in hit_wallets:
        event_id, inserted = insert_buy_event(conn, wallet, event, observed_at_ms)
        if not event_id:
            continue
        if inserted:
            inserted_count += 1
            print(
                f"wallet_buy wallet={wallet} mint={mint} "
                f"signature={text_field(event, 'signature')}",
                file=sys.stderr,
                flush=True,
            )
        for match in create_matches(conn, event_id, match_window_ms):
            match_count += 1
            print(
                f"mint_match mint={match['mint']} wallets="
                f"{match['first_wallet']},{match['second_wallet']} "
                f"signatures={match['first_signature']},{match['second_signature']}",
                file=sys.stderr,
                flush=True,
            )
            if ensure_high_tracker(conn, match, high_track_ms):
                print(
                    f"mint_10m_track_start mint={match['mint']} "
                    f"until_ms={int(match['matched_at_ms']) + high_track_ms}",
                    file=sys.stderr,
                    flush=True,
                )
            update_active_highs_for_event(conn, event, observed_at_ms)
    conn.commit()
    return inserted_count, match_count


def decode_event(raw: bytes) -> dict | None:
    try:
        event = orjson.loads(raw)
    except orjson.JSONDecodeError as exc:
        print(f"skip invalid json: {exc}", file=sys.stderr, flush=True)
        return None
    return event if isinstance(event, dict) else None


def replay_file(
    conn: sqlite3.Connection,
    path: str,
    watch_wallets: set[str],
    watch_actions: set[str],
    match_window_ms: int,
    high_track_ms: int,
) -> tuple[int, int]:
    inserted = 0
    matches = 0
    with open(path, "rb") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            event = decode_event(line)
            if not event:
                continue
            event_inserted, event_matches = process_event(
                conn, event, watch_wallets, watch_actions, match_window_ms, high_track_ms
            )
            inserted += event_inserted
            matches += event_matches
    return inserted, matches


def subscribe_forever(
    conn: sqlite3.Connection,
    endpoint: str,
    watch_wallets: set[str],
    watch_actions: set[str],
    match_window_ms: int,
    high_track_ms: int,
) -> None:
    ctx = zmq.Context.instance()
    sub = ctx.socket(zmq.SUB)
    sub.setsockopt(zmq.RCVHWM, env_int("ZMQ_RCVHWM", 10000))
    sub.connect(endpoint)
    # Empty subscription receives raw one-frame events and normalized multipart
    # messages. This recorder only processes raw one-frame PumpAPI events.
    sub.setsockopt_string(zmq.SUBSCRIBE, "")
    print(
        f"wallet watcher subscribed {endpoint}; wallets={','.join(sorted(watch_wallets))}",
        file=sys.stderr,
        flush=True,
    )

    while True:
        parts = sub.recv_multipart()
        if len(parts) != 1:
            continue
        event = decode_event(parts[0])
        if not event:
            continue
        process_event(
            conn, event, watch_wallets, watch_actions, match_window_ms, high_track_ms
        )


def list_matches(conn: sqlite3.Connection, limit: int) -> None:
    rows = conn.execute(
        """
        SELECT *
        FROM mint_wallet_matches
        ORDER BY matched_at_ms DESC
        LIMIT ?
        """,
        (limit,),
    ).fetchall()
    for row in rows:
        print(orjson.dumps(dict(row)).decode(), flush=True)


def list_highs(conn: sqlite3.Connection, limit: int) -> None:
    rows = conn.execute(
        """
        SELECT *
        FROM mint_10m_highs
        ORDER BY first_matched_at_ms DESC
        LIMIT ?
        """,
        (limit,),
    ).fetchall()
    for row in rows:
        print(orjson.dumps(dict(row)).decode(), flush=True)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Record watched PumpAPI wallet buys and same-mint matches."
    )
    parser.add_argument("--db", default=os.getenv("WALLET_WATCH_DB", DEFAULT_DB))
    parser.add_argument(
        "--endpoint",
        default=os.getenv("ZMQ_ENDPOINT", DEFAULT_ENDPOINT),
        help="ZMQ endpoint published by pumpapi_zmq_bridge.py",
    )
    parser.add_argument(
        "--wallets",
        default=os.getenv("WATCH_WALLETS", DEFAULT_WALLETS),
        help="Comma separated wallet pubkeys to watch",
    )
    parser.add_argument(
        "--actions",
        default=os.getenv("WATCH_ACTIONS", "buy"),
        help="Comma separated PumpAPI actions to record; default: buy",
    )
    parser.add_argument(
        "--match-window-seconds",
        type=float,
        default=env_float("WALLET_MATCH_WINDOW_SECONDS", 3600),
    )
    parser.add_argument(
        "--high-track-seconds",
        type=float,
        default=env_float("HIGH_TRACK_SECONDS", DEFAULT_HIGH_TRACK_SECONDS),
        help="Seconds to track price/market-cap highs after a mint match; default: 600",
    )
    parser.add_argument("--replay-file", help="Replay a JSONL raw PumpAPI capture file")
    parser.add_argument("--init-db", action="store_true", help="Create/upgrade DB and exit")
    parser.add_argument("--list-matches", type=int, metavar="LIMIT")
    parser.add_argument("--list-highs", type=int, metavar="LIMIT")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    watch_wallets = set(parse_csv(args.wallets))
    if len(watch_wallets) < 2:
        print("WATCH_WALLETS requires at least two wallet pubkeys", file=sys.stderr)
        return 2
    watch_actions = {action.lower() for action in parse_csv(args.actions)}
    match_window_ms = int(args.match_window_seconds * 1000)
    high_track_ms = int(args.high_track_seconds * 1000)

    conn = open_db(args.db)
    if args.init_db:
        print(f"wallet watch db ready: {args.db}", file=sys.stderr, flush=True)
        return 0
    if args.list_matches is not None:
        list_matches(conn, args.list_matches)
        return 0
    if args.list_highs is not None:
        list_highs(conn, args.list_highs)
        return 0

    seeded = seed_recent_high_trackers(conn, high_track_ms)
    if seeded:
        print(f"seeded recent high trackers={seeded}", file=sys.stderr, flush=True)

    if args.replay_file:
        inserted, matches = replay_file(
            conn,
            args.replay_file,
            watch_wallets,
            watch_actions,
            match_window_ms,
            high_track_ms,
        )
        print(
            f"replay done inserted={inserted} matches={matches} db={args.db}",
            file=sys.stderr,
            flush=True,
        )
        return 0

    subscribe_forever(
        conn, args.endpoint, watch_wallets, watch_actions, match_window_ms, high_track_ms
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
