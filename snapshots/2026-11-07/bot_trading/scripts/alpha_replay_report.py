#!/usr/bin/env python3
"""Read-only replay report for Alpha paper signals."""
from __future__ import annotations

import argparse
import json
import sqlite3
import statistics
import time
from pathlib import Path


WINDOWS = (30_000, 120_000, 600_000)


def number(value, *names):
    for name in names:
        item = value.get(name)
        if isinstance(item, (int, float)):
            return float(item)
        if isinstance(item, str):
            try:
                return float(item)
            except ValueError:
                pass
    return None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--database", default="/root/solana/bot_trading/data/bot_trading.sqlite")
    parser.add_argument("--hours", type=float, default=24.0)
    parser.add_argument("--output", default="/root/solana/bot_trading/data/alpha_replay_report.json")
    parser.add_argument("--max-signals", type=int, default=1000)
    args = parser.parse_args()
    since = int((time.time() - args.hours * 3600) * 1000)
    db = sqlite3.connect(f"file:{Path(args.database).resolve()}?mode=ro", uri=True)
    signals = db.execute(
        "SELECT signal_id,mint,pool,pool_id,created_at_ms FROM alpha_paper_signals "
        "WHERE created_at_ms >= ? ORDER BY created_at_ms DESC LIMIT ?", (since, args.max_signals)
    ).fetchall()
    results = []
    for signal_id, mint, pool, pool_id, created_at in signals:
        rows = db.execute(
            "SELECT received_at_ms,raw_json FROM alpha_observation_events "
            "WHERE mint=? AND (? IS NULL OR pool_id=?) AND received_at_ms>? AND received_at_ms<=? "
            "ORDER BY received_at_ms",
            (mint, pool_id, pool_id, created_at, created_at + WINDOWS[-1]),
        ).fetchall()
        parsed = []
        for ts, raw in rows:
            try:
                event = json.loads(raw)
            except (TypeError, json.JSONDecodeError):
                continue
            quote = number(event, "quote_amount", "quoteAmount", "quote_amount_raw") or 0.0
            side = str(event.get("tx_type") or event.get("txType") or event.get("action") or "").lower()
            price = number(event, "priceSol", "price")
            parsed.append((ts, side, quote, price))
        metrics = {}
        for window in WINDOWS:
            within = [row for row in parsed if row[0] <= created_at + window]
            buy = sum(row[2] for row in within if row[1] == "buy")
            sell = sum(row[2] for row in within if row[1] == "sell")
            prices = [row[3] for row in within if row[3] and row[3] > 0]
            metrics[str(window)] = {
                "events": len(within),
                "net_quote_flow": buy - sell,
                "price_return": (prices[-1] / prices[0] - 1.0) if len(prices) >= 2 else None,
            }
        results.append({"signal_id": signal_id, "mint": mint, "pool": pool, "metrics": metrics})

    summary = {"signals": len(results)}
    for window in WINDOWS:
        values = [r["metrics"][str(window)]["net_quote_flow"] for r in results]
        returns = [r["metrics"][str(window)]["price_return"] for r in results if r["metrics"][str(window)]["price_return"] is not None]
        summary[str(window)] = {
            "with_followup_events": sum(r["metrics"][str(window)]["events"] > 0 for r in results),
            "positive_net_flow_rate": sum(v > 0 for v in values) / len(values) if values else None,
            "median_net_quote_flow": statistics.median(values) if values else None,
            "price_samples": len(returns),
            "median_price_return": statistics.median(returns) if returns else None,
        }
    report = {"generated_at_ms": int(time.time() * 1000), "window_hours": args.hours, "summary": summary}
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
