#!/usr/bin/env python3
"""Replay PumpAPI observation JSONL into paper Alpha signals.

This tool is deliberately offline: it never calls trade_service and never signs
or submits a transaction.  It is used to validate event quality and the first
windowed flow signal before enabling live execution.
"""

import argparse
import json
import math
import sys
from collections import defaultdict, deque
from pathlib import Path


WINDOWS_MS = (30_000, 120_000, 600_000)


def number(value):
    if isinstance(value, bool) or value is None:
        return None
    try:
        result = float(value)
    except (TypeError, ValueError):
        return None
    return result if math.isfinite(result) else None


def event_time(event):
    return number(event.get("event_time_ms")) or number(event.get("received_at_ms"))


def event_key(event):
    return (
        event.get("mint"),
        event.get("pool_id") or event.get("event_pool"),
    )


def quote_amount(event):
    value = event.get("quote_amount")
    if value is None:
        value = event.get("quoteAmount")
    return number(value) or 0.0


def side(event):
    value = event.get("action") or event.get("tx_type") or event.get("txType")
    return value.lower() if isinstance(value, str) else ""


def features(events, as_of_ms, window_ms):
    start = as_of_ms - window_ms
    selected = [event for event in events if start <= (event_time(event) or 0) <= as_of_ms]
    buys = [event for event in selected if side(event) == "buy"]
    sells = [event for event in selected if side(event) == "sell"]
    buy_flow = sum(quote_amount(event) for event in buys)
    sell_flow = sum(quote_amount(event) for event in sells)
    traders = {event.get("tx_signer") or event.get("signer") for event in selected}
    traders.discard(None)
    trader_buys = defaultdict(float)
    for event in buys:
        signer = event.get("tx_signer") or event.get("signer")
        if signer:
            trader_buys[signer] += quote_amount(event)
    max_trader_buy = max(trader_buys.values(), default=0.0)
    concentration = max_trader_buy / buy_flow if buy_flow > 0 else 0.0
    return {
        "window_ms": window_ms,
        "buy_count": len(buys),
        "sell_count": len(sells),
        "buy_flow": buy_flow,
        "sell_flow": sell_flow,
        "net_flow": buy_flow - sell_flow,
        "buy_sell_ratio": (len(buys) + 1) / (len(sells) + 1),
        "unique_traders": len(traders),
        "concentration": concentration,
    }


def score(short, medium, long):
    """Return a bounded, explainable first-pass score."""
    flow_base = max(abs(medium["buy_flow"]) / 4.0, 1e-12)
    flow_acceleration = max(-3.0, min(3.0, short["net_flow"] / flow_base))
    ratio = max(0.0, min(3.0, medium["buy_sell_ratio"])) / 3.0
    trader = max(0.0, min(10.0, medium["unique_traders"])) / 10.0
    concentration_penalty = max(0.0, min(1.0, medium["concentration"]))
    persistence = 1.0 if long["net_flow"] > 0 else 0.0
    value = (
        0.45 * (flow_acceleration + 3.0) / 6.0
        + 0.20 * ratio
        + 0.20 * trader
        + 0.15 * persistence
        - 0.20 * concentration_penalty
    )
    return max(0.0, min(1.0, value))


def replay(events, min_score=0.65, min_traders=3):
    grouped = defaultdict(list)
    seen = set()
    stats = defaultdict(int)
    signals = []
    for event in sorted(events, key=lambda item: event_time(item) or 0):
        stats["events_read"] += 1
        event_id = event.get("event_id")
        if not event_id or event_id in seen:
            stats["events_duplicate"] += 1
            continue
        seen.add(event_id)
        if event.get("route_status") != "executable":
            stats["events_observation_only"] += 1
            continue
        if side(event) not in {"buy", "sell"} or not event.get("mint"):
            stats["events_non_trade"] += 1
            continue
        timestamp = event_time(event)
        if timestamp is None:
            stats["events_missing_time"] += 1
            continue
        key = event_key(event)
        grouped[key].append(event)
        if side(event) != "buy":
            continue
        short = features(grouped[key], timestamp, WINDOWS_MS[0])
        medium = features(grouped[key], timestamp, WINDOWS_MS[1])
        long = features(grouped[key], timestamp, WINDOWS_MS[2])
        value = score(short, medium, long)
        if (
            value >= min_score
            and short["net_flow"] > 0
            and medium["unique_traders"] >= min_traders
            and medium["concentration"] <= 0.35
        ):
            signal_id = f"{event['event_id']}:alpha-v1"
            signals.append(
                {
                    "signal_id": signal_id,
                    "event_id": event["event_id"],
                    "mint": event["mint"],
                    "pool": event.get("trade_service_pool") or event.get("event_pool"),
                    "pool_id": event.get("pool_id"),
                    "side": "buy",
                    "score": round(value, 6),
                    "as_of_ms": int(timestamp),
                    "features_30s": short,
                    "features_2m": medium,
                    "features_10m": long,
                    "mode": "paper",
                }
            )
            stats["signals"] += 1
    stats["mints_seen"] = len(grouped)
    return signals, dict(stats)


def read_jsonl(paths):
    for path in paths:
        with Path(path).open("rb") as stream:
            for line_number, raw in enumerate(stream, 1):
                if not raw.strip():
                    continue
                try:
                    value = json.loads(raw)
                except json.JSONDecodeError as exc:
                    print(f"skip invalid JSON {path}:{line_number}: {exc}", file=sys.stderr)
                    continue
                if isinstance(value, dict):
                    yield value


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("input", nargs="+", help="observation JSONL spool files")
    parser.add_argument("--output", help="paper signal JSONL output; stdout by default")
    parser.add_argument("--min-score", type=float, default=0.65)
    parser.add_argument("--min-traders", type=int, default=3)
    args = parser.parse_args()
    signals, stats = replay(read_jsonl(args.input), args.min_score, args.min_traders)
    output = open(args.output, "w", encoding="utf-8") if args.output else sys.stdout
    try:
        for signal in signals:
            output.write(json.dumps(signal, separators=(",", ":"), sort_keys=True))
            output.write("\n")
    finally:
        if args.output:
            output.close()
    print(json.dumps({"stats": stats}, sort_keys=True), file=sys.stderr)


if __name__ == "__main__":
    main()
