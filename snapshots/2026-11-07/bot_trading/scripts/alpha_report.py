#!/usr/bin/env python3
"""Read-only Alpha paper-signal report."""
from __future__ import annotations

import argparse
import json
import sqlite3
import statistics
import time
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--database", default="/root/solana/bot_trading/data/bot_trading.sqlite")
    parser.add_argument("--hours", type=float, default=24.0)
    parser.add_argument("--output", default="/root/solana/bot_trading/data/alpha_report.json")
    args = parser.parse_args()

    since_ms = int((time.time() - args.hours * 3600) * 1000)
    conn = sqlite3.connect(f"file:{Path(args.database).resolve()}?mode=ro", uri=True)
    observations = conn.execute(
        "SELECT COUNT(*), COUNT(DISTINCT mint), COUNT(DISTINCT pool) "
        "FROM alpha_observation_events WHERE received_at_ms >= ?", (since_ms,)
    ).fetchone()
    rows = conn.execute(
        "SELECT score, pool, mint FROM alpha_paper_signals "
        "WHERE created_at_ms >= ? ORDER BY score DESC", (since_ms,)
    ).fetchall()
    scores = [float(row[0]) for row in rows]
    by_pool = {}
    by_mint = {}
    for score, pool, mint in rows:
        by_pool[pool or "unknown"] = by_pool.get(pool or "unknown", 0) + 1
        by_mint[mint] = by_mint.get(mint, 0) + 1
    report = {
        "generated_at_ms": int(time.time() * 1000),
        "window_hours": args.hours,
        "since_ms": since_ms,
        "observations": {
            "events": observations[0],
            "mints": observations[1],
            "pools": observations[2],
        },
        "paper_signals": {
            "count": len(rows),
            "score_min": min(scores) if scores else None,
            "score_max": max(scores) if scores else None,
            "score_mean": statistics.mean(scores) if scores else None,
            "score_median": statistics.median(scores) if scores else None,
            "by_pool": dict(sorted(by_pool.items(), key=lambda item: (-item[1], item[0]))),
            "top_mints": [
                {"mint": mint, "signals": count}
                for mint, count in sorted(by_mint.items(), key=lambda item: (-item[1], item[0]))[:20]
            ],
        },
    }
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
