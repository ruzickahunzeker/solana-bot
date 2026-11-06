#!/usr/bin/env python3
"""Verify one complete Jupiter Shadow run without modifying its SQLite DB."""

from __future__ import annotations

import argparse
import json
import math
import re
import sqlite3
import sys
from datetime import datetime, timezone
from pathlib import Path


START_RE = re.compile(
    r"(?P<ts>\S+) shadow_started=true duration_seconds=(?P<duration>\d+) "
    r"interval_seconds=(?P<interval>\d+)"
)
SAMPLE_RE = re.compile(
    r"(?P<ts>\S+) http_status=(?P<http>\d+) "
    r"schema=(?P<schema>\S+)(?: result_status=(?P<status>\S+))?"
)
COMPLETE_RE = re.compile(r"(?P<ts>\S+) shadow_complete=true samples=(?P<samples>\d+)")
MAX_SAMPLE_INTERVAL_SECONDS = 60
MAX_METRIC_DELAY_MS = 31_000


def parse_ts(value: str) -> datetime:
    return datetime.fromisoformat(value.replace("Z", "+00:00")).astimezone(timezone.utc)


def percentile(values: list[int], fraction: float) -> int | None:
    if not values:
        return None
    values = sorted(values)
    index = min(len(values) - 1, max(0, math.ceil(len(values) * fraction) - 1))
    return values[index]


def output_quality(rows: list[tuple]) -> dict[str, int | float | None]:
    quoted = [row for row in rows if row[0] == "quoted"]
    checked = [row for row in quoted if row[4] is not None and row[5] is not None]
    passed = sum(
        1
        for row in checked
        if (expected := row[4]) > 0
        and (minimum := row[5]) > 0
        and minimum <= expected
        and (slippage := row[6]) is not None
        and 1 <= slippage <= 3000
        and minimum >= expected * (10_000 - slippage) // 10_000
    )
    total = len(quoted)
    return {
        "min_output_checked_count": len(checked),
        "min_output_missing_count": total - len(checked),
        "min_output_pass_count": passed,
        "min_output_coverage_rate": len(checked) / total if total else None,
        "min_output_pass_rate": passed / total if total else None,
    }


def fail(message: str, summary: dict[str, object] | None = None) -> int:
    output = {"passed": False, "error": message}
    if summary:
        output.update(summary)
    print(json.dumps(output, ensure_ascii=False, sort_keys=True))
    return 2


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--log", type=Path, required=True)
    parser.add_argument("--db", type=Path, required=True)
    parser.add_argument("--min-quoted-rate", type=float, default=0.99)
    parser.add_argument("--max-p95-ms", type=int, default=3000)
    parser.add_argument("--max-p99-ms", type=int, default=10000)
    args = parser.parse_args()
    if not 0 <= args.min_quoted_rate <= 1 or args.max_p95_ms < 0 or args.max_p99_ms < 0:
        parser.error("invalid Shadow acceptance threshold")

    lines = args.log.read_text(encoding="utf-8").splitlines()
    starts = [START_RE.fullmatch(line) for line in lines]
    starts = [match for match in starts if match]
    if len(starts) != 1 or sum("shadow_started=true" in line for line in lines) != 1:
        return fail("log must contain exactly one shadow_started record")
    start_match = starts[0]
    start = parse_ts(start_match.group("ts"))
    duration = int(start_match.group("duration"))
    interval = int(start_match.group("interval"))
    samples = [SAMPLE_RE.fullmatch(line) for line in lines]
    samples = [match for match in samples if match]
    complete = [COMPLETE_RE.fullmatch(line) for line in lines]
    complete = [match for match in complete if match]
    summary: dict[str, object] = {
        "started_at": start.isoformat(),
        "configured_duration_seconds": duration,
        "configured_interval_seconds": interval,
        "log_sample_count": len(samples),
        "complete": bool(complete),
    }
    if duration < 86_400:
        return fail("configured shadow duration is shorter than 24 hours", summary)
    if interval <= 0:
        return fail("configured shadow interval must be positive", summary)
    if interval > MAX_SAMPLE_INTERVAL_SECONDS:
        return fail("configured shadow interval exceeds one minute", summary)
    if len(samples) != sum("http_status=" in line for line in lines):
        return fail("shadow contains a malformed HTTP sample record", summary)
    if len(complete) != 1 or sum("shadow_complete=true" in line for line in lines) != 1:
        return fail("shadow_complete record is missing or duplicated", summary)
    complete_match = complete[0]
    end = parse_ts(complete_match.group("ts"))
    declared_samples = int(complete_match.group("samples"))
    summary["ended_at"] = end.isoformat()
    summary["elapsed_seconds"] = int((end - start).total_seconds())
    if declared_samples != len(samples):
        return fail("declared sample count does not match log records", summary)
    if (end - start).total_seconds() < duration:
        return fail("shadow window is shorter than configured duration", summary)
    if not samples:
        return fail("shadow has no samples", summary)
    timestamps = [parse_ts(match.group("ts")) for match in samples]
    if any(right <= left for left, right in zip(timestamps, timestamps[1:])):
        return fail("shadow sample timestamps are not strictly increasing", summary)
    gaps = [int((right - left).total_seconds()) for left, right in zip(timestamps, timestamps[1:])]
    max_gap = max(gaps, default=0)
    summary["max_sample_gap_seconds"] = max_gap
    first_gap = (timestamps[0] - start).total_seconds()
    last_gap = (end - timestamps[-1]).total_seconds()
    summary["first_sample_delay_seconds"] = first_gap
    summary["last_sample_to_end_seconds"] = last_gap
    if first_gap < 0 or first_gap > interval + 30 or last_gap < 0 or last_gap > interval + 30:
        return fail("shadow run has a missing start or end sample", summary)
    # Permit scheduling jitter, but reject a missing multi-minute segment.
    if max_gap > interval + 30:
        return fail("shadow sample interval has an unexplained gap", summary)
    bad = [
        match
        for match in samples
        if match.group("http") != "200"
        or match.group("schema") != "ok"
        or match.group("status") not in {"quoted", "failed", "unavailable"}
    ]
    summary["bad_log_samples"] = len(bad)
    if bad:
        return fail("shadow contains non-success HTTP or invalid schema samples", summary)

    try:
        connection = sqlite3.connect(f"file:{args.db.resolve()}?mode=ro", uri=True)
    except sqlite3.Error as error:
        return fail(f"cannot open staging database read-only: {error}", summary)
    with connection:
        connection.execute("PRAGMA query_only=ON")
        tables = {
            row[0]
            for row in connection.execute("SELECT name FROM sqlite_master WHERE type='table'")
        }
        required = {"aggregator_shadow_metrics", "aggregator_submissions", "aggregator_quotes"}
        if missing := sorted(required - tables):
            return fail(f"missing staging tables: {', '.join(missing)}", summary)
        start_ms = int(start.timestamp() * 1000)
        end_ms = int(end.timestamp() * 1000)
        rows = connection.execute(
            """SELECT status, latency_ms, error_class, created_at_ms,
                      expected_output_units, minimum_output_units, slippage_bps
                 FROM aggregator_shadow_metrics
                WHERE provider='jupiter' AND created_at_ms BETWEEN ? AND ?
                ORDER BY created_at_ms, rowid""",
            (start_ms, end_ms),
        ).fetchall()
        in_flight = connection.execute(
            "SELECT COUNT(*) FROM aggregator_submissions WHERE status IN ('reserved','submitting')"
        ).fetchone()[0]
        if in_flight:
            return fail(f"staging DB has {in_flight} in-flight submissions", summary)
        run_submissions = connection.execute(
            "SELECT COUNT(*) FROM aggregator_submissions WHERE created_at_ms BETWEEN ? AND ?",
            (start_ms, end_ms),
        ).fetchone()[0]
        run_quotes = connection.execute(
            "SELECT COUNT(*) FROM aggregator_quotes WHERE created_at_ms BETWEEN ? AND ?",
            (start_ms, end_ms),
        ).fetchone()[0]
        summary["run_quotes"] = run_quotes
        summary["run_submissions"] = run_submissions
        if run_quotes or run_submissions:
            return fail("read-only Shadow window contains quote or submission state", summary)
        sensitive = ("http://", "https://", "ws://", "wss://", "api-key=", "token=")
        for (_, _, error_class, _, _, _, _) in rows:
            if error_class and any(marker in error_class.lower() for marker in sensitive):
                return fail("staging metrics contain an unsanitized endpoint or credential", summary)
    if len(rows) != len(samples):
        summary["db_metric_count"] = len(rows)
        return fail("Jupiter metric count does not match this run's log samples", summary)
    for index, (sample, row) in enumerate(zip(samples, rows)):
        sample_ms = int(parse_ts(sample.group("ts")).timestamp() * 1000)
        metric_ms = int(row[3])
        if row[0] != sample.group("status"):
            return fail(f"Jupiter metric status differs from log sample {index}", summary)
        if not sample_ms <= metric_ms <= sample_ms + MAX_METRIC_DELAY_MS:
            return fail(f"Jupiter metric time differs from log sample {index}", summary)
    if any(row[1] is None or int(row[1]) < 0 for row in rows):
        return fail("Jupiter latency metrics contain missing or negative values", summary)
    latencies = [int(row[1]) for row in rows]
    status_counts: dict[str, int] = {}
    for row in rows:
        status = row[0]
        status_counts[status] = status_counts.get(status, 0) + 1
    log_status_counts: dict[str, int] = {}
    for match in samples:
        status = match.group("status")
        assert status is not None
        log_status_counts[status] = log_status_counts.get(status, 0) + 1
    summary["log_status_counts"] = log_status_counts
    if status_counts != log_status_counts:
        return fail("persisted statuses do not match this run's log samples", summary)
    summary.update(output_quality(rows))
    if summary["min_output_pass_count"] != log_status_counts.get("quoted", 0):
        return fail("Jupiter quoted metrics have missing or invalid minimum output", summary)
    quoted_rate = log_status_counts.get("quoted", 0) / len(samples)
    summary["quoted_rate"] = quoted_rate
    if quoted_rate < args.min_quoted_rate:
        return fail("Jupiter quoted rate is below threshold", summary)
    summary.update(
        {
            "db_metric_count": len(rows),
            "status_counts": status_counts,
            "p50_latency_ms": percentile(latencies, 0.50),
            "p95_latency_ms": percentile(latencies, 0.95),
            "p99_latency_ms": percentile(latencies, 0.99),
            "in_flight_submissions": 0,
        }
    )
    if summary["p95_latency_ms"] is None or summary["p99_latency_ms"] is None:
        return fail("Jupiter latency metrics are missing", summary)
    if summary["p95_latency_ms"] > args.max_p95_ms or summary["p99_latency_ms"] > args.max_p99_ms:
        return fail("Jupiter latency exceeds threshold", summary)
    summary["passed"] = True
    print(json.dumps(summary, ensure_ascii=False, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())

