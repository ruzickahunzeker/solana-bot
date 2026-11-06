import sqlite3
import tempfile
import unittest
from pathlib import Path

from audit_jupiter_shadow_progress import audit


class ShadowProgressTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.log = root / "shadow.log"
        self.db = root / "trades.sqlite"
        self.log.write_text(
            "2026-01-01T00:00:00Z shadow_started=true duration_seconds=86400 interval_seconds=60\n"
            "2026-01-01T00:00:01Z http_status=200 schema=ok result_status=quoted\n"
            "2026-01-01T00:01:01Z http_status=200 schema=ok result_status=quoted\n"
        )
        with sqlite3.connect(self.db) as connection:
            connection.executescript(
                "CREATE TABLE aggregator_shadow_metrics(provider TEXT,status TEXT,latency_ms INTEGER,error_class TEXT,created_at_ms INTEGER,expected_output_units INTEGER,minimum_output_units INTEGER,slippage_bps INTEGER);"
                "CREATE TABLE aggregator_quotes(created_at_ms INTEGER);"
                "CREATE TABLE aggregator_submissions(created_at_ms INTEGER,status TEXT);"
            )
            connection.executemany(
                "INSERT INTO aggregator_shadow_metrics VALUES ('jupiter','quoted',20,NULL,?,100,99,100)",
                [(1767225602000,), (1767225662000,)],
            )

    def test_incomplete_run_can_be_audited_without_passing_final_acceptance(self):
        result = audit(self.log, self.db)
        self.assertTrue(result["passed_so_far"])
        self.assertFalse(result["complete"])
        self.assertEqual(result["samples"], 2)
        self.assertEqual(result["db_metrics"], 2)
        self.assertEqual(result["min_output_checked_count"], 2)
        self.assertEqual(result["min_output_pass_count"], 2)
        self.assertEqual(result["min_output_coverage_rate"], 1.0)

    def test_duplicate_metric_is_rejected(self):
        with sqlite3.connect(self.db) as connection:
            connection.execute(
                "INSERT INTO aggregator_shadow_metrics VALUES ('jupiter','quoted',20,NULL,1767225663000,100,99,100)"
            )
        with self.assertRaisesRegex(ValueError, "metric count differs"):
            audit(self.log, self.db)

    def test_gap_is_rejected(self):
        self.log.write_text(
            self.log.read_text().replace("00:01:01Z", "00:03:01Z")
        )
        with self.assertRaisesRegex(ValueError, "gap or duplicate"):
            audit(self.log, self.db)

    def test_malformed_sample_is_rejected(self):
        self.log.write_text(self.log.read_text() + "broken http_status=000\n")
        with self.assertRaisesRegex(ValueError, "malformed HTTP sample"):
            audit(self.log, self.db)

    def test_missing_minimum_output_is_rejected(self):
        with sqlite3.connect(self.db) as connection:
            connection.execute(
                "UPDATE aggregator_shadow_metrics SET minimum_output_units=NULL WHERE rowid=1"
            )
        with self.assertRaisesRegex(ValueError, "missing or invalid minimum output"):
            audit(self.log, self.db)


if __name__ == "__main__":
    unittest.main()
