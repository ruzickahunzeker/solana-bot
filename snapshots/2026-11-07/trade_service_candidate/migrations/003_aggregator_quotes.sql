CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS aggregator_quotes (
    quote_id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    provider_quote_id TEXT NOT NULL,
    wallet_id TEXT NOT NULL,
    wallet_pubkey TEXT NOT NULL,
    input_mint TEXT NOT NULL,
    output_mint TEXT NOT NULL,
    amount_lamports INTEGER NOT NULL,
    expected_output_units INTEGER NOT NULL,
    minimum_output_units INTEGER NOT NULL,
    slippage_bps INTEGER NOT NULL,
    price_impact_percent TEXT NOT NULL,
    route_json TEXT NOT NULL,
    route_fingerprint TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    message_hash TEXT,
    expires_at_ms INTEGER NOT NULL,
    status TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_aggregator_quotes_wallet_expiry
    ON aggregator_quotes(wallet_id, expires_at_ms);
CREATE INDEX IF NOT EXISTS idx_aggregator_quotes_provider_route
    ON aggregator_quotes(provider, route_fingerprint);

CREATE TABLE IF NOT EXISTS aggregator_submissions (
    submission_id TEXT PRIMARY KEY,
    quote_id TEXT NOT NULL,
    wallet_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    message_hash TEXT,
    status TEXT NOT NULL,
    response_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE(wallet_id, idempotency_key),
    FOREIGN KEY(quote_id) REFERENCES aggregator_quotes(quote_id)
);

CREATE INDEX IF NOT EXISTS idx_aggregator_submissions_quote
    ON aggregator_submissions(quote_id, created_at_ms);

CREATE TABLE IF NOT EXISTS aggregator_shadow_metrics (
    metric_id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    wallet_id TEXT NOT NULL,
    input_mint TEXT NOT NULL,
    output_mint TEXT NOT NULL,
    amount_lamports INTEGER NOT NULL,
    slippage_bps INTEGER NOT NULL,
    status TEXT NOT NULL,
    latency_ms INTEGER NOT NULL,
    expected_output_units INTEGER,
    minimum_output_units INTEGER,
    route_fingerprint TEXT,
    price_impact_percent TEXT,
    error_class TEXT,
    created_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_aggregator_shadow_metrics_provider_time
    ON aggregator_shadow_metrics(provider, created_at_ms);

INSERT OR IGNORE INTO schema_migrations(version, applied_at_ms)
VALUES (3, CAST(strftime('%s','now') AS INTEGER) * 1000);
