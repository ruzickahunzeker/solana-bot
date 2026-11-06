#!/usr/bin/env python3
"""Offline acceptance tests for the read-only Jupiter Shadow verifier."""

import contextlib
import io
import json
import sqlite3
import sys
import tempfile
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path
from unittest.mock import patch

import verify_jupiter_shadow


class JupiterShadowVerificationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.log = Path(self.temp.name) / "run.log"
        self.db = Path(self.temp.name) / "trades.sqlite"
        self.start = datetime(2026, 1, 1, tzinfo=timezone.utc)
        self.samples = 1440
        self.lines = [
            f"{self.iso(self.start)} shadow_started=true duration_seconds=86400 interval_seconds=60"
        ]
        connection = sqlite3.connect(self.db)
        connection.executescript(
            """CREATE TABLE aggregator_shadow_metrics (
                 provider TEXT, status TEXT, latency_ms INTEGER,
                 error_class TEXT, created_at_ms INTEGER,
                 expected_output_units INTEGER, minimum_output_units INTEGER,
                 slippage_bps INTEGER);
               CREATE TABLE aggregator_submissions (
                 status TEXT, created_at_ms INTEGER);
               CREATE TABLE aggregator_quotes (created_at_ms INTEGER);"""
        )
        for index in range(self.samples):
            timestamp = self.start + timedelta(seconds=index * 60)
            self.lines.append(
                f"{self.iso(timestamp)} http_status=200 schema=ok result_status=quoted"
            )
            connection.execute(
                "INSERT INTO aggregator_shadow_metrics VALUES ('jupiter','quoted',20,NULL,?,100,99,100)",
                (int(timestamp.timestamp() * 1000),),
            )
        connection.commit()
        connection.close()
        self.finish()

    @staticmethod
    def iso(value):
        return value.isoformat().replace("+00:00", "Z")

    def finish(self):
        self.lines = [line for line in self.lines if "shadow_complete=true" not in line]
        end = self.start + timedelta(days=1)
        self.lines.append(f"{self.iso(end)} shadow_complete=true samples={self.samples}")
        self.log.write_text("\n".join(self.lines) + "\n")

    def verify(self):
        stdout = io.StringIO()
        with patch.object(sys, "argv", ["verify", "--log", str(self.log), "--db", str(self.db)]):
            with contextlib.redirect_stdout(stdout):
                status = verify_jupiter_shadow.main()
        return status, json.loads(stdout.getvalue())

    def test_accepts_one_complete_read_only_window(self):
        status, report = self.verify()
        self.assertEqual(status, 0)
        self.assertTrue(report["passed"])
        self.assertEqual(report["db_metric_count"], self.samples)
        self.assertEqual(report["min_output_checked_count"], self.samples)
        self.assertEqual(report["min_output_pass_count"], self.samples)
        self.assertEqual(report["min_output_coverage_rate"], 1.0)

    def test_rejects_incomplete_window(self):
        self.log.write_text("\n".join(self.lines[:-1]) + "\n")
        status, report = self.verify()
        self.assertEqual(status, 2)
        self.assertFalse(report["complete"])

    def test_rejects_malformed_http_sample_even_if_other_records_match(self):
        self.lines.insert(100, "2026-01-01T01:40:00Z http_status=000 malformed")
        self.finish()
        status, report = self.verify()
        self.assertEqual(status, 2)
        self.assertIn("malformed HTTP", report["error"])

    def test_rejects_malformed_duplicate_completion(self):
        self.lines.append("2026-01-02T00:00:00Z shadow_complete=true samples=1440 trailing")
        self.log.write_text("\n".join(self.lines) + "\n")
        status, report = self.verify()
        self.assertEqual(status, 2)
        self.assertIn("shadow_complete", report["error"])

    def test_rejects_a_missing_sample_even_when_count_is_rewritten(self):
        self.lines.pop(700)
        self.samples -= 1
        self.finish()
        status, report = self.verify()
        self.assertEqual(status, 2)
        self.assertIn("gap", report["error"])

    def test_rejects_missing_persisted_metric(self):
        connection = sqlite3.connect(self.db)
        connection.execute("DELETE FROM aggregator_shadow_metrics WHERE rowid=1")
        connection.commit()
        connection.close()
        status, report = self.verify()
        self.assertEqual(status, 2)
        self.assertIn("metric count", report["error"])

    def test_rejects_quote_state_in_read_only_window(self):
        connection = sqlite3.connect(self.db)
        connection.execute(
            "INSERT INTO aggregator_quotes VALUES (?)", (int(self.start.timestamp() * 1000),)
        )
        connection.commit()
        connection.close()
        status, report = self.verify()
        self.assertEqual(status, 2)
        self.assertIn("read-only", report["error"])

    def test_rejects_low_quoted_rate_even_with_matching_metrics(self):
        for index in range(1, 51):
            self.lines[index] = self.lines[index].replace("result_status=quoted", "result_status=failed")
        self.finish()
        connection = sqlite3.connect(self.db)
        connection.execute("UPDATE aggregator_shadow_metrics SET status='failed' WHERE rowid<=50")
        connection.commit()
        connection.close()
        status, report = self.verify()
        self.assertEqual(status, 2)
        self.assertIn("quoted rate", report["error"])

    def test_rejects_metric_with_wrong_sample_time(self):
        connection = sqlite3.connect(self.db)
        timestamp = int((self.start + timedelta(seconds=32)).timestamp() * 1000)
        connection.execute(
            "UPDATE aggregator_shadow_metrics SET created_at_ms=? WHERE rowid=1",
            (timestamp,),
        )
        connection.commit()
        connection.close()
        status, report = self.verify()
        self.assertEqual(status, 2)
        self.assertIn("metric time", report["error"])

    def test_rejects_metric_with_wrong_sample_status(self):
        connection = sqlite3.connect(self.db)
        connection.execute("UPDATE aggregator_shadow_metrics SET status='failed' WHERE rowid=1")
        connection.commit()
        connection.close()
        status, report = self.verify()
        self.assertEqual(status, 2)
        self.assertIn("metric status", report["error"])

    def test_rejects_missing_latency_in_matching_metric(self):
        connection = sqlite3.connect(self.db)
        connection.execute("UPDATE aggregator_shadow_metrics SET latency_ms=NULL WHERE rowid=1")
        connection.commit()
        connection.close()
        status, report = self.verify()
        self.assertEqual(status, 2)
        self.assertIn("latency metrics", report["error"])

    def test_rejects_quoted_metric_with_missing_minimum_output(self):
        with sqlite3.connect(self.db) as connection:
            connection.execute(
                "UPDATE aggregator_shadow_metrics SET minimum_output_units=NULL WHERE rowid=1"
            )
        status, report = self.verify()
        self.assertEqual(status, 2)
        self.assertEqual(report["min_output_missing_count"], 1)
        self.assertLess(report["min_output_pass_rate"], 1.0)

    def test_rejects_quoted_metric_below_slippage_floor(self):
        with sqlite3.connect(self.db) as connection:
            connection.execute(
                "UPDATE aggregator_shadow_metrics SET minimum_output_units=98 WHERE rowid=1"
            )
        status, report = self.verify()
        self.assertEqual(status, 2)
        self.assertEqual(report["min_output_checked_count"], self.samples)
        self.assertEqual(report["min_output_pass_count"], self.samples - 1)

    def test_rejects_reordered_log_samples(self):
        self.lines[100], self.lines[101] = self.lines[101], self.lines[100]
        self.finish()
        status, report = self.verify()
        self.assertEqual(status, 2)
        self.assertIn("strictly increasing", report["error"])

    def test_rejects_hourly_sampling_configuration(self):
        self.lines[0] = self.lines[0].replace("interval_seconds=60", "interval_seconds=3600")
        self.finish()
        status, report = self.verify()
        self.assertEqual(status, 2)
        self.assertIn("exceeds one minute", report["error"])


if __name__ == "__main__":
    unittest.main()

