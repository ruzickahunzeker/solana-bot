#!/usr/bin/env python3
"""Standalone SQLite DDL checks; actual SQLx migration/scan acceptance stays in Rust CI."""

from pathlib import Path
from contextlib import closing
import sqlite3
import tempfile
import unittest

from test_sdk_candidate_migration import apply_upgrade, fixture, old_rows, schema

ROOT = Path(__file__).resolve().parents[1]
MARKER = "010_ordinary_sdk_observation_scan"
INDEX = "idx_ordinary_sdk_observation_scan"
DDL = (ROOT / "migrations" / f"{MARKER}.sql").read_text(encoding="utf-8")


def setup(path=":memory:"):
    db = fixture(path)
    apply_upgrade(db)
    return db


def apply_index(db):
    """Model SQLite DDL+marker transactions, not a replacement for the Rust validator."""
    db.execute("BEGIN")
    try:
        marked = db.execute("SELECT COUNT(*) FROM ordinary_submission_migrations WHERE name=?", (MARKER,)).fetchone()[0]
        existing = db.execute("SELECT type,tbl_name,sql FROM sqlite_schema WHERE name=?", (INDEX,)).fetchone()
        if not marked:
            if existing is not None:
                raise ValueError("refusing unmarked index adoption")
            db.execute(DDL)
        with closing(sqlite3.connect(":memory:")) as reference:
            reference.execute("CREATE TABLE ordinary_execution_claims(created_at_ms,request_id,schema_version,operation,owner_nonce,sdk_prepared_json,unsigned_message,unsigned_sha256,signed_signature,signed_transaction,signed_sha256,final_observation_json,submission_stage)")
            reference.execute(DDL)
            expected = reference.execute("SELECT type,tbl_name,sql FROM sqlite_schema WHERE name=?", (INDEX,)).fetchone()
        if db.execute("SELECT type,tbl_name,sql FROM sqlite_schema WHERE name=?", (INDEX,)).fetchone() != expected:
            raise ValueError("refusing missing or changed index repair")
        if not marked:
            db.execute("INSERT INTO ordinary_submission_migrations VALUES(?)", (MARKER,))
        db.commit()
    except Exception:
        db.rollback()
        raise


class ObservationIndexTests(unittest.TestCase):
    def setUp(self):
        self.db = setup()

    def tearDown(self):
        self.db.close()

    def test_preserves_original_rows_parent_and_legacy_index(self):
        before = old_rows(self.db)
        parents = self.db.execute("SELECT * FROM trades ORDER BY request_id").fetchall()
        legacy = self.db.execute("SELECT sql FROM sqlite_schema WHERE name='idx_ordinary_signed_observation_scan'").fetchone()
        apply_index(self.db)
        self.assertEqual(old_rows(self.db), before)
        self.assertEqual(self.db.execute("SELECT * FROM trades ORDER BY request_id").fetchall(), parents)
        self.assertEqual(self.db.execute("SELECT sql FROM sqlite_schema WHERE name='idx_ordinary_signed_observation_scan'").fetchone(), legacy)
        self.assertEqual(self.db.execute("PRAGMA foreign_key_check").fetchall(), [])
        columns = self.db.execute(f"PRAGMA index_xinfo({INDEX})").fetchall()
        self.assertEqual([(r[2], r[3], r[4], r[5]) for r in columns], [("created_at_ms", 0, "BINARY", 1), ("request_id", 0, "BINARY", 1), (None, 0, "BINARY", 0)])

    def test_repeat_is_idempotent(self):
        apply_index(self.db)
        before = schema(self.db)
        for _ in range(3):
            apply_index(self.db)
            self.assertEqual(schema(self.db), before)

    def test_marker_failure_rolls_back_index_creation(self):
        self.db.execute("CREATE TRIGGER deny_index_marker BEFORE INSERT ON ordinary_submission_migrations WHEN NEW.name='010_ordinary_sdk_observation_scan' BEGIN SELECT RAISE(ABORT,'fixture marker failure'); END")
        before = schema(self.db)
        with self.assertRaises(sqlite3.IntegrityError):
            apply_index(self.db)
        self.assertEqual(schema(self.db), before)
        self.db.execute("DROP TRIGGER deny_index_marker")
        apply_index(self.db)

    def test_correct_unmarked_index_is_not_adopted(self):
        self.db.execute(DDL)
        before = schema(self.db)
        with self.assertRaises(ValueError):
            apply_index(self.db)
        self.assertEqual(schema(self.db), before)

    def test_marked_missing_or_altered_index_is_not_repaired(self):
        apply_index(self.db)
        self.db.execute(f"DROP INDEX {INDEX}")
        for altered in (False, True):
            if altered:
                self.db.execute(f"CREATE INDEX {INDEX} ON ordinary_execution_claims(request_hash)")
            before = schema(self.db)
            with self.assertRaises(ValueError):
                apply_index(self.db)
            self.assertEqual(schema(self.db), before)

    def test_file_reopen_keeps_original_rows_and_marker(self):
        with tempfile.TemporaryDirectory(prefix="sdk-observation-index-") as directory:
            path = str(Path(directory) / "fixture.sqlite")
            db = setup(path)
            apply_index(db)
            before = old_rows(db)
            db.close()
            db = sqlite3.connect(path, isolation_level=None)
            try:
                db.execute("PRAGMA foreign_keys=ON")
                apply_index(db)
                self.assertEqual(old_rows(db), before)
                self.assertEqual(db.execute("PRAGMA integrity_check").fetchone()[0], "ok")
            finally:
                db.close()


if __name__ == "__main__":
    print(f"SQLite {sqlite3.sqlite_version}: standalone SDK index DDL tests only")
    unittest.main(verbosity=2)
