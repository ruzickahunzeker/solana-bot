#!/usr/bin/env python3
"""SQLite DDL evidence only; not SQLx, SDK, API, or Rust integration acceptance."""

from pathlib import Path
import sqlite3
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
MARKER = "009_ordinary_sdk_prepared_candidate"
UPGRADE = (ROOT / "migrations" / f"{MARKER}.sql").read_text(encoding="utf-8")


def fixture(path=":memory:"):
    db = sqlite3.connect(path, isolation_level=None)
    db.execute("PRAGMA foreign_keys=ON")
    for name in (
        "001_trade_persistence", "004_ordinary_execution_claims",
        "005_ordinary_signed_submissions", "006_ordinary_final_observations",
        "007_ordinary_observation_scan",
    ):
        db.executescript((ROOT / "migrations" / f"{name}.sql").read_text(encoding="utf-8"))
    for i, stage in enumerate(("claimed", "preparing", "prepared", "signing", "signed", "submitted", "unknown", "failed")):
        request_id = f"tr_fixture_{i}"
        db.execute(
            "INSERT INTO trades(request_id,wallet_id,wallet_pubkey,idempotency_key,request_hash,"
            "request_json,side,pool,mint,execution_method,status,created_at_ms,updated_at_ms) "
            "VALUES(?,'public-wallet','public-key',?,'original-hash','{}','buy','pump-amm',"
            "'public-mint','sdk','unknown',1,2)", (request_id, f" literal-key-{i} "),
        )
        db.execute(
            "INSERT INTO ordinary_execution_claims(request_id,schema_version,operation,request_hash,"
            "created_at_ms,owner_nonce,submission_stage,bound_quote_json,preflight_json,unsigned_message,"
            "unsigned_sha256,signed_signature,signed_transaction,signed_sha256,submission_error,final_observation_json) "
            "VALUES(?,1,'ordinary-sol-buy','original-hash',?,?,?,' {\"fixture\":1} ', '{}',?,"
            "'unsigned-hash','public-signature',?,'signed-hash',?,?)",
            (request_id, i, None if i == 0 else "a" * 32, stage,
             bytes((0, 255, i)), bytes((255, 0, i)), "error\x00suffix", None if i == 0 else "{}"),
        )
    # Prepare an 008-shaped fixture. This does not execute or claim to test
    # the Rust migrator's preflight, comparison or rollback implementation.
    db.executescript((ROOT / "migrations/008_ordinary_claim_operations.sql").read_text(encoding="utf-8"))
    db.execute("DROP TABLE ordinary_execution_claims")
    db.execute("ALTER TABLE ordinary_execution_claims_operation_upgrade RENAME TO ordinary_execution_claims")
    db.executescript((ROOT / "migrations/007_ordinary_observation_scan.sql").read_text(encoding="utf-8"))
    db.execute("CREATE TABLE ordinary_submission_migrations(name TEXT PRIMARY KEY NOT NULL)")
    for name in ("005_ordinary_signed_submissions", "006_ordinary_final_observations", "007_ordinary_observation_scan", "008_ordinary_claim_operations"):
        db.execute("INSERT INTO ordinary_submission_migrations VALUES(?)", (name,))
    return db


def apply_upgrade(db):
    """Exercise SQLite's transactional ALTER+marker semantics, not a Rust port."""
    db.execute("BEGIN")
    try:
        if not db.execute("SELECT COUNT(*) FROM ordinary_submission_migrations WHERE name=?", (MARKER,)).fetchone()[0]:
            # execute(), not executescript(): the latter would implicitly commit
            # the transaction under Python's sqlite3 API.
            db.execute(UPGRADE)
            db.execute("INSERT INTO ordinary_submission_migrations VALUES(?)", (MARKER,))
        db.commit()
    except Exception:
        db.rollback()
        raise


def old_rows(db):
    columns = [r[1] for r in db.execute("PRAGMA table_info(ordinary_execution_claims)") if r[1] != "sdk_prepared_json"]
    projection = ",".join(f'"{c}",typeof("{c}")' for c in columns)
    return db.execute(f"SELECT {projection} FROM ordinary_execution_claims ORDER BY request_id").fetchall()


def schema(db):
    return db.execute("SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type,name").fetchall()


class CandidateMigrationTests(unittest.TestCase):
    def setUp(self):
        self.db = fixture()

    def tearDown(self):
        self.db.close()

    def test_preserves_every_old_value_type_parent_and_index(self):
        rows = old_rows(self.db)
        parents = self.db.execute("SELECT * FROM trades ORDER BY request_id").fetchall()
        indexes = self.db.execute("SELECT name,sql FROM sqlite_schema WHERE type='index' ORDER BY name").fetchall()
        foreign_keys = self.db.execute("PRAGMA foreign_key_list(ordinary_execution_claims)").fetchall()
        apply_upgrade(self.db)
        self.assertEqual(old_rows(self.db), rows)
        self.assertEqual(self.db.execute("SELECT * FROM trades ORDER BY request_id").fetchall(), parents)
        self.assertEqual(self.db.execute("SELECT name,sql FROM sqlite_schema WHERE type='index' ORDER BY name").fetchall(), indexes)
        self.assertEqual(self.db.execute("PRAGMA foreign_key_list(ordinary_execution_claims)").fetchall(), foreign_keys)
        self.assertEqual(self.db.execute("PRAGMA foreign_key_check").fetchall(), [])

    def test_new_column_is_nullable_and_empty_for_all_legacy_stages(self):
        apply_upgrade(self.db)
        column = next(r for r in self.db.execute("PRAGMA table_info(ordinary_execution_claims)") if r[1] == "sdk_prepared_json")
        self.assertEqual((column[2], column[3], column[4]), ("TEXT", 0, None))
        self.assertEqual(self.db.execute("SELECT COUNT(*) FROM ordinary_execution_claims WHERE sdk_prepared_json IS NOT NULL").fetchone()[0], 0)

    def test_marker_failure_rolls_back_schema_and_rows_then_allows_retry(self):
        self.db.execute("CREATE TRIGGER deny_marker BEFORE INSERT ON ordinary_submission_migrations WHEN NEW.name='009_ordinary_sdk_prepared_candidate' BEGIN SELECT RAISE(ABORT,'fixture marker failure'); END")
        before, rows = schema(self.db), old_rows(self.db)
        with self.assertRaises(sqlite3.IntegrityError):
            apply_upgrade(self.db)
        self.assertEqual(schema(self.db), before)
        self.assertEqual(old_rows(self.db), rows)
        self.db.execute("DROP TRIGGER deny_marker")
        apply_upgrade(self.db)
        self.assertEqual(self.db.execute("SELECT COUNT(*) FROM ordinary_submission_migrations WHERE name=?", (MARKER,)).fetchone()[0], 1)

    def test_repeated_migration_preserves_existing_opaque_metadata_and_nonce(self):
        apply_upgrade(self.db)
        # DDL continuity only: this opaque text is not a valid SDK prepared record.
        self.db.execute("UPDATE ordinary_execution_claims SET sdk_prepared_json='opaque fixture' WHERE request_id='tr_fixture_1'")
        before = self.db.execute("SELECT * FROM ordinary_execution_claims ORDER BY request_id").fetchall()
        for _ in range(3):
            apply_upgrade(self.db)
            self.assertEqual(self.db.execute("SELECT * FROM ordinary_execution_claims ORDER BY request_id").fetchall(), before)
        self.assertEqual(self.db.execute("SELECT COUNT(*) FROM ordinary_submission_migrations WHERE name=?", (MARKER,)).fetchone()[0], 1)

    def test_unmarked_existing_column_is_not_silently_adopted(self):
        self.db.execute(UPGRADE)
        before = schema(self.db)
        with self.assertRaises(sqlite3.OperationalError):
            apply_upgrade(self.db)
        self.assertEqual(schema(self.db), before)
        self.assertEqual(self.db.execute("SELECT COUNT(*) FROM ordinary_submission_migrations WHERE name=?", (MARKER,)).fetchone()[0], 0)

    def test_file_reopen_preserves_the_original_rows_and_marker(self):
        with tempfile.TemporaryDirectory(prefix="sdk-migration-") as directory:
            path = str(Path(directory) / "fixture.sqlite")
            db = fixture(path)
            try:
                apply_upgrade(db)
                before = old_rows(db)
            finally:
                db.close()
            db = sqlite3.connect(path, isolation_level=None)
            try:
                db.execute("PRAGMA foreign_keys=ON")
                apply_upgrade(db)
                self.assertEqual(old_rows(db), before)
                self.assertEqual(db.execute("PRAGMA integrity_check").fetchone()[0], "ok")
                self.assertEqual(db.execute("PRAGMA foreign_key_check").fetchall(), [])
            finally:
                db.close()


if __name__ == "__main__":
    print(f"SQLite {sqlite3.sqlite_version}: standalone DDL tests only", flush=True)
    unittest.main(verbosity=2)
