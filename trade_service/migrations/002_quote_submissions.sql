-- Separate from ordinary trades: no fabricated Pool or mixed amount semantics.
CREATE TABLE IF NOT EXISTS quote_submissions (
    submission_id TEXT PRIMARY KEY NOT NULL,
    wallet_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    intent_json TEXT NOT NULL,
    preflight_json TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('reserved','signed','submitted','unknown','failed')),
    signature TEXT,
    signed_transaction BLOB,
    error TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE(wallet_id, idempotency_key),
    CHECK ((signature IS NULL) = (signed_transaction IS NULL)),
    CHECK (status NOT IN ('signed','submitted') OR signature IS NOT NULL)
);
