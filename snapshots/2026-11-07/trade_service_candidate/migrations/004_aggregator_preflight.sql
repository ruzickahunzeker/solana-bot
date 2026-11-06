-- SQLite has no portable IF NOT EXISTS form for ADD COLUMN. The startup
-- migration helper checks pragma_table_info and adds these columns once.
INSERT OR IGNORE INTO schema_migrations(version, applied_at_ms)
VALUES (4, CAST(strftime('%s','now') AS INTEGER) * 1000);
