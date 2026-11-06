#!/usr/bin/env python3
"""Audit an in-progress Jupiter Shadow run without changing its acceptance state."""

from __future__ import annotations

import argparse
import json
import sqlite3
from collections import Counter
from pathlib import Path

from verify_jupiter_shadow import (
    MAX_METRIC_DELAY_MS,
    SAMPLE_RE,
    START_RE,
    output_quality,
    parse_ts,
    percentile,
)


def audit(log_path: Path, db_path: Path) -> dict:
    # The log is snapshotted once so the active sampler can append safely.
    lines = log_path.read_text(encoding="utf-8").splitlines()
    starts = [match for line in lines if (match := START_RE.fullmatch(line))]
    if len(starts) != 1:
        raise ValueError("expected exactly one shadow_started record")
    start_ms = int(parse_ts(starts[0].group("ts")).timestamp() * 1000)
    interval = int(starts[0].group("interval"))
    if interval <= 0 or interval > 60:
        raise ValueError("invalid sampler interval")
    samples = [match for line in lines if (match := SAMPLE_RE.fullmatch(line))]
    if len(samples) != sum("http_status=" in line for line in lines):
        raise ValueError("malformed HTTP sample record")
    if not samples:
        raise ValueError("no completed log samples")
    sample_times = [int(parse_ts(item.group("ts")).timestamp() * 1000) for item in samples]
    gaps_ms = [right - left for left, right in zip(sample_times, sample_times[1:])]
    if sample_times[0] < start_ms or sample_times[0] - start_ms > (interval + 30) * 1000:
        raise ValueError("missing first sample")
    if any(gap <= 0 or gap > (interval + 30) * 1000 for gap in gaps_ms):
        raise ValueError("sample sequence has a gap or duplicate")
    if any(
        item.group("http") != "200"
        or item.group("schema") != "ok"
        or item.group("status") not in {"quoted", "failed", "unavailable"}
        for item in samples
    ):
        raise ValueError("HTTP, schema or result status failed")

    end_ms = sample_times[-1] + MAX_METRIC_DELAY_MS
    connection = sqlite3.connect(f"file:{db_path.resolve()}?mode=ro", uri=True)
    try:
        connection.execute("PRAGMA query_only=ON")
        rows = connection.execute(
            """SELECT status,latency_ms,error_class,created_at_ms,
                      expected_output_units,minimum_output_units,slippage_bps
                 FROM aggregator_shadow_metrics
                WHERE provider='jupiter' AND created_at_ms BETWEEN ? AND ?
                ORDER BY created_at_ms,rowid""",
            (start_ms, end_ms),
        ).fetchall()
        quotes = connection.execute(
            "SELECT COUNT(*) FROM aggregator_quotes WHERE created_at_ms BETWEEN ? AND ?",
            (start_ms, end_ms),
        ).fetchone()[0]
        submissions = connection.execute(
            "SELECT COUNT(*) FROM aggregator_submissions WHERE created_at_ms BETWEEN ? AND ?",
            (start_ms, end_ms),
        ).fetchone()[0]
        in_flight = connection.execute(
            "SELECT COUNT(*) FROM aggregator_submissions WHERE status IN ('reserved','submitting')"
        ).fetchone()[0]
    finally:
        connection.close()
    if len(rows) != len(samples):
        raise ValueError("persisted Jupiter metric count differs from log")
    for index, (sample, sample_ms, row) in enumerate(zip(samples, sample_times, rows)):
        status, latency, error_class, metric_ms = row[:4]
        if status != sample.group("status"):
            raise ValueError(f"metric {index} status differs from log")
        if not sample_ms <= metric_ms <= sample_ms + MAX_METRIC_DELAY_MS:
            raise ValueError(f"metric {index} timestamp differs from log")
        if latency is None or latency < 0:
            raise ValueError(f"metric {index} has invalid latency")
        if error_class and any(
            marker in error_class.lower()
            for marker in ("http://", "https://", "ws://", "wss://", "api-key=", "token=")
        ):
            raise ValueError(f"metric {index} contains an unsanitized endpoint")
    if quotes or submissions or in_flight:
        raise ValueError("read-only Shadow has quote/submission state")
    latencies = [row[1] for row in rows]
    status_counts = dict(Counter(row[0] for row in rows))
    quality = output_quality(rows)
    if quality["min_output_pass_count"] != status_counts.get("quoted", 0):
        raise ValueError("Jupiter quoted metrics have missing or invalid minimum output")
    return {
        "passed_so_far": True,
        "complete": any("shadow_complete=true" in line for line in lines),
        "started_at": starts[0].group("ts"),
        "last_sample_at": samples[-1].group("ts"),
        "samples": len(samples),
        "db_metrics": len(rows),
        "status_counts": status_counts,
        "quoted_rate": status_counts.get("quoted", 0) / len(rows),
        "max_gap_seconds": max(gaps_ms, default=0) / 1000,
        "p50_latency_ms": percentile(latencies, 0.50),
        "p95_latency_ms": percentile(latencies, 0.95),
        "p99_latency_ms": percentile(latencies, 0.99),
        "quotes": quotes,
        "submissions": submissions,
        "in_flight": in_flight,
        **quality,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--log", type=Path, required=True)
    parser.add_argument("--db", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = audit(args.log, args.db)
    except (OSError, sqlite3.Error, ValueError) as error:
        print(json.dumps({"passed_so_far": False, "error": str(error)}, sort_keys=True))
        return 2
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
