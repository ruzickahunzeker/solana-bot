CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS trades (
    request_id TEXT PRIMARY KEY,
    wallet_id TEXT NOT NULL,
    wallet_pubkey TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    request_json TEXT NOT NULL,
    side TEXT NOT NULL,
    pool TEXT NOT NULL,
    mint TEXT NOT NULL,
    pool_id TEXT,
    execution_method TEXT NOT NULL,
    status TEXT NOT NULL,
    signature TEXT,
    error TEXT,
    simulation_json TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE(wallet_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS idx_trades_created_at ON trades(created_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_trades_signature ON trades(signature);

INSERT OR IGNORE INTO schema_migrations(version, applied_at_ms)
VALUES (1, CAST(strftime('%s','now') AS INTEGER) * 1000);
