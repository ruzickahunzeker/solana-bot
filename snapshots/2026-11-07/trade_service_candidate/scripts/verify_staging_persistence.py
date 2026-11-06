#!/usr/bin/env python3
"""Read-only checks for an isolated candidate SQLite database after restart."""

import os
import sqlite3
import sys


def fail(message: str) -> "NoReturn":
    raise SystemExit(message)


path = os.environ.get("CANDIDATE_STAGING_DB")
if not path:
    fail("set CANDIDATE_STAGING_DB to the isolated candidate SQLite path")

try:
    connection = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
except sqlite3.Error as error:
    fail(f"cannot open staging database read-only: {error}")

with connection:
    tables = {
        row[0]
        for row in connection.execute(
            "SELECT name FROM sqlite_master WHERE type='table'"
        )
    }
    required = {
        "aggregator_quotes",
        "aggregator_submissions",
        "aggregator_shadow_metrics",
    }
    missing = sorted(required - tables)
    if missing:
        fail(f"missing aggregator tables: {', '.join(missing)}")

    expected_columns = {
        "aggregator_quotes": {
            "quote_id",
            "provider",
            "request_hash",
            "route_fingerprint",
            "wallet_pubkey",
            "status",
            "expires_at_ms",
        },
        "aggregator_submissions": {
            "idempotency_key",
            "quote_id",
            "status",
        },
        "aggregator_shadow_metrics": {
            "provider",
            "wallet_id",
            "status",
            "latency_ms",
        },
    }
    for table, expected in expected_columns.items():
        columns = {
            row[1] for row in connection.execute(f"PRAGMA table_info({table})")
        }
        missing_columns = sorted(expected - columns)
        if missing_columns:
            fail(f"{table} missing columns: {', '.join(missing_columns)}")

    in_flight = connection.execute(
        "SELECT COUNT(*) FROM aggregator_submissions "
        "WHERE status IN ('reserved', 'submitting')"
    ).fetchone()[0]
    if in_flight:
        fail(f"restart recovery left {in_flight} reserved/submitting submissions")

    # Error text is diagnostic data, but it must never retain provider/RPC
    # endpoints or query credentials after restart.
    sensitive_markers = ("http://", "https://", "ws://", "wss://", "api-key=", "token=")
    for table, column in (
        ("aggregator_submissions", "response_json"),
        ("aggregator_shadow_metrics", "error_class"),
    ):
        columns = {
            row[1] for row in connection.execute(f"PRAGMA table_info({table})")
        }
        if column not in columns:
            continue
        for (value,) in connection.execute(
            f"SELECT {column} FROM {table} WHERE {column} IS NOT NULL"
        ):
            text = str(value).lower()
            if any(marker in text for marker in sensitive_markers):
                fail(f"{table}.{column} contains an unsanitized URL or credential marker")

print("staging persistence readback passed: aggregator schema present and no in-flight reservations")
