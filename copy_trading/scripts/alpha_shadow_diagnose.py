#!/usr/bin/env python3
"""Read-only diagnosis of Alpha shadow orders.

The command deliberately opens the large copy-trading database read-only and
only scans ``alpha_shadow_orders``.  It separates local admission blocks from
Trade Service responses, then groups remote failures by pool, instruction
index, error code, mint and pool id.  No rows, WAL files or indexes are
modified.
"""

from __future__ import annotations

import argparse
import json
import re
import sqlite3
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any


INSTRUCTION_ERROR = re.compile(
    r"InstructionError\((?P<index>\d+),\s*Custom\((?P<code>\d+)\)\)"
)


def connect_readonly(path: Path) -> sqlite3.Connection:
    uri = f"file:{path.resolve()}?mode=ro"
    connection = sqlite3.connect(uri, uri=True)
    connection.execute("PRAGMA query_only=ON")
    return connection


def classify(row: sqlite3.Row) -> tuple[str, str]:
    error = (row["error"] or "").strip()
    status = (row["status"] or "").strip()
    if error == "alpha shadow per-minute submission limit reached":
        return "local_rate_limit", "local admission block"
    if status == "blocked" and not row["trade_service_request_id"]:
        return "local_block", "local admission or readiness block"
    if error:
        match = INSTRUCTION_ERROR.search(error)
        if match:
            return (
                "remote_instruction_error",
                f"instruction={match.group('index')} custom={match.group('code')}",
            )
        return "remote_or_client_error", error[:240]
    return status or "unknown", "no error text"


def diagnose(connection: sqlite3.Connection, limit: int) -> dict[str, Any]:
    rows = connection.execute(
        """
        SELECT id, signal_id, mint, side, pool, pool_id, quote_mint, amount_sol,
               status, request_json, response_json, trade_service_request_id,
               error, created_at_ms, updated_at_ms
          FROM alpha_shadow_orders
         ORDER BY created_at_ms ASC, id ASC
        """
    ).fetchall()
    counts: Counter[str] = Counter()
    detail_counts: Counter[str] = Counter()
    pool_counts: defaultdict[str, Counter[str]] = defaultdict(Counter)
    instruction_counts: Counter[str] = Counter()
    remote_mint_counts: Counter[tuple[str, str, str | None]] = Counter()
    samples: list[dict[str, Any]] = []
    for row in rows:
        category, detail = classify(row)
        counts[category] += 1
        detail_counts[detail] += 1
        pool = row["pool"] or "unknown"
        pool_counts[pool][category] += 1
        match = INSTRUCTION_ERROR.search(row["error"] or "")
        if match:
            instruction_counts[
                f"instruction={match.group('index')} custom={match.group('code')}"
            ] += 1
        if category not in {"local_rate_limit", "local_block"}:
            remote_mint_counts[(pool, row["mint"] or "unknown", row["pool_id"])] += 1
        if len(samples) < limit and category not in {"local_rate_limit", "local_block"}:
            samples.append(
                {
                    "id": row["id"],
                    "status": row["status"],
                    "category": category,
                    "detail": detail,
                    "pool": pool,
                    "pool_id": row["pool_id"],
                    "mint": row["mint"],
                    "quote_mint": row["quote_mint"],
                    "amount_sol": row["amount_sol"],
                    "request_id": row["trade_service_request_id"],
                    "error": row["error"],
                    "request_json": row["request_json"],
                    "response_json": row["response_json"],
                    "created_at_ms": row["created_at_ms"],
                }
            )

    remote_total = sum(
        count
        for category, count in counts.items()
        if category.startswith("remote_")
    )
    request_id_total = sum(1 for row in rows if row["trade_service_request_id"])
    return {
        "read_only": True,
        "order_count": len(rows),
        "trade_service_request_id_count": request_id_total,
        "remote_attempt_count": remote_total,
        "categories": dict(counts),
        "details": detail_counts.most_common(30),
        "by_pool": {pool: dict(values) for pool, values in sorted(pool_counts.items())},
        "instruction_errors": instruction_counts.most_common(),
        "remote_mint_pool_groups": [
            {"pool": pool, "mint": mint, "pool_id": pool_id, "count": count}
            for (pool, mint, pool_id), count in remote_mint_counts.most_common()
        ][: max(limit, 1)],
        "samples": samples,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("database", type=Path, help="copy_trading SQLite database")
    parser.add_argument("--sample-limit", type=int, default=20)
    args = parser.parse_args()
    if args.sample_limit < 1:
        parser.error("--sample-limit must be greater than zero")
    with connect_readonly(args.database) as connection:
        connection.row_factory = sqlite3.Row
        print(json.dumps(diagnose(connection, args.sample_limit), ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
