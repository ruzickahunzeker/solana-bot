#!/usr/bin/env python3
"""Print an auditable summary of Alpha observation and paper signal data.

This tool is read-only. It opens the SQLite database in URI mode with
immutable=1 when possible and never calls Trade Service or changes rows.
"""

from __future__ import annotations

import argparse
import json
import sqlite3
import time
from pathlib import Path
from typing import Any


def connect_readonly(path: Path) -> sqlite3.Connection:
    uri = f"file:{path.resolve()}?mode=ro"
    return sqlite3.connect(uri, uri=True)


def scalar(connection: sqlite3.Connection, query: str, args: tuple[Any, ...]) -> Any:
    row = connection.execute(query, args).fetchone()
    return row[0] if row else None


def build_report(connection: sqlite3.Connection, since_ms: int, limit: int) -> dict[str, Any]:
    tables = {
        row[0]
        for row in connection.execute(
            "SELECT name FROM sqlite_master WHERE type='table'"
        ).fetchall()
    }
    required = {"alpha_observation_events", "alpha_paper_signals"}
    missing_tables = required - tables
    shadow_ready = "alpha_shadow_orders" in tables
    signal_columns = (
        {
            row[1]
            for row in connection.execute("PRAGMA table_info(alpha_paper_signals)").fetchall()
        }
        if "alpha_paper_signals" in tables
        else set()
    )
    required_signal_columns = {
        "signal_id",
        "event_id",
        "mint",
        "pool",
        "pool_id",
        "quote_mint",
        "side",
        "score",
        "status",
        "created_at_ms",
        "order_id",
        "trade_service_request_id",
        "remote_status",
        "updated_at_ms",
    }
    missing_columns = required_signal_columns - signal_columns
    if missing_tables or missing_columns:
        return {
            "mode": "paper-report",
            "read_only": True,
            "since_ms": since_ms,
            "schema_ready": False,
            "missing_tables": sorted(missing_tables),
            "missing_shadow_table": not shadow_ready,
            "missing_signal_columns": sorted(missing_columns) if "alpha_paper_signals" in tables else [],
            "observation_count": 0,
            "paper_signal_count": 0,
            "signals_by_status": [],
            "signals_by_pool": [],
            "recent_signals": [],
            "shadow_order_count": 0,
            "shadow_orders_by_status": [],
            "shadow_request_id_coverage": 0.0,
        }
    signal_count = scalar(
        connection,
        "SELECT COUNT(*) FROM alpha_paper_signals WHERE created_at_ms >= ?",
        (since_ms,),
    )
    observation_count = scalar(
        connection,
        "SELECT COUNT(*) FROM alpha_observation_events WHERE received_unix_ms >= ?",
        (since_ms,),
    )
    rows = connection.execute(
        """SELECT status, COUNT(*) AS count, AVG(score) AS avg_score,
                         MIN(score) AS min_score, MAX(score) AS max_score
                  FROM alpha_paper_signals
                 WHERE created_at_ms >= ?
                 GROUP BY status ORDER BY status""",
        (since_ms,),
    ).fetchall()
    by_status = [
        {
            "status": row[0],
            "count": row[1],
            "avg_score": row[2],
            "min_score": row[3],
            "max_score": row[4],
        }
        for row in rows
    ]
    pool_rows = connection.execute(
        """SELECT COALESCE(pool, 'unknown') AS pool, COUNT(*) AS count,
                         AVG(score) AS avg_score
                  FROM alpha_paper_signals
                 WHERE created_at_ms >= ?
                 GROUP BY pool ORDER BY count DESC, pool""",
        (since_ms,),
    ).fetchall()
    by_pool = [
        {"pool": row[0], "count": row[1], "avg_score": row[2]}
        for row in pool_rows
    ]
    recent_rows = connection.execute(
        """SELECT signal_id, event_id, mint, pool, pool_id, quote_mint, side, score,
                         status, created_at_ms, order_id,
                         trade_service_request_id, remote_status, updated_at_ms
                  FROM alpha_paper_signals
                 WHERE created_at_ms >= ?
                 ORDER BY created_at_ms DESC, signal_id DESC LIMIT ?""",
        (since_ms, limit),
    ).fetchall()
    recent = [
        {
            "signal_id": row[0],
            "event_id": row[1],
            "mint": row[2],
            "pool": row[3],
            "pool_id": row[4],
            "quote_mint": row[5],
            "side": row[6],
            "score": row[7],
            "status": row[8],
            "created_at_ms": row[9],
            "order_id": row[10],
            "trade_service_request_id": row[11],
            "remote_status": row[12],
            "updated_at_ms": row[13],
        }
        for row in recent_rows
    ]
    shadow_order_count = 0
    shadow_by_status: list[dict[str, Any]] = []
    shadow_request_id_coverage = 0.0
    if shadow_ready:
        shadow_order_count = int(
            scalar(
                connection,
                "SELECT COUNT(*) FROM alpha_shadow_orders WHERE created_at_ms >= ?",
                (since_ms,),
            )
            or 0
        )
        shadow_rows = connection.execute(
            """SELECT status, COUNT(*) AS count
                 FROM alpha_shadow_orders
                WHERE created_at_ms >= ?
                GROUP BY status ORDER BY status""",
            (since_ms,),
        ).fetchall()
        shadow_by_status = [{"status": row[0], "count": row[1]} for row in shadow_rows]
        if shadow_order_count:
            with_request_id = scalar(
                connection,
                """SELECT COUNT(*) FROM alpha_shadow_orders
                    WHERE created_at_ms >= ? AND trade_service_request_id IS NOT NULL""",
                (since_ms,),
            )
            shadow_request_id_coverage = float(with_request_id or 0) / shadow_order_count
    return {
        "mode": "paper-report",
        "read_only": True,
        "schema_ready": True,
        "missing_shadow_table": not shadow_ready,
        "since_ms": since_ms,
        "observation_count": observation_count,
        "paper_signal_count": signal_count,
        "signals_by_status": by_status,
        "signals_by_pool": by_pool,
        "recent_signals": recent,
        "shadow_order_count": shadow_order_count,
        "shadow_orders_by_status": shadow_by_status,
        "shadow_request_id_coverage": shadow_request_id_coverage,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("database", type=Path, help="copy_trading SQLite database")
    parser.add_argument(
        "--since-ms",
        type=int,
        default=0,
        help="only include rows at or after this Unix timestamp in milliseconds",
    )
    parser.add_argument("--last-hours", type=float, help="convenience window relative to now")
    parser.add_argument("--limit", type=int, default=20, help="number of recent signals")
    parser.add_argument(
        "--check-shadow",
        action="store_true",
        help="exit non-zero unless shadow schema and status gates pass",
    )
    parser.add_argument(
        "--min-request-id-coverage",
        type=float,
        default=1.0,
        help="required request_id coverage when --check-shadow is used",
    )
    parser.add_argument(
        "--max-unknown",
        type=int,
        default=0,
        help="maximum unknown shadow orders allowed by --check-shadow",
    )
    parser.add_argument(
        "--max-blocked",
        type=int,
        default=0,
        help="maximum blocked shadow orders allowed by --check-shadow",
    )
    args = parser.parse_args()
    if args.limit < 1:
        parser.error("--limit must be greater than zero")
    if not 0.0 <= args.min_request_id_coverage <= 1.0:
        parser.error("--min-request-id-coverage must be between 0 and 1")
    if args.max_unknown < 0 or args.max_blocked < 0:
        parser.error("--max-unknown and --max-blocked must not be negative")
    since_ms = args.since_ms
    if args.last_hours is not None:
        if args.last_hours < 0:
            parser.error("--last-hours must not be negative")
        since_ms = int(time.time() * 1000 - args.last_hours * 3_600_000)
    with connect_readonly(args.database) as connection:
        connection.execute("PRAGMA query_only=ON")
        report = build_report(connection, since_ms, args.limit)
        print(json.dumps(report, ensure_ascii=False, indent=2))
    if args.check_shadow:
        by_status = {
            item["status"]: item["count"] for item in report["shadow_orders_by_status"]
        }
        failures = []
        if not report["schema_ready"] or report.get("missing_shadow_table", True):
            failures.append("shadow schema is not ready")
        if report["shadow_request_id_coverage"] < args.min_request_id_coverage:
            failures.append("request_id coverage is below threshold")
        if by_status.get("unknown", 0) > args.max_unknown:
            failures.append("unknown shadow orders exceed threshold")
        if by_status.get("blocked", 0) > args.max_blocked:
            failures.append("blocked shadow orders exceed threshold")
        if failures:
            raise SystemExit("shadow check failed: " + "; ".join(failures))


if __name__ == "__main__":
    main()
