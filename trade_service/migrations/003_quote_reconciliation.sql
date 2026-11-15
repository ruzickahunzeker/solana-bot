ALTER TABLE quote_submissions RENAME TO quote_submissions_v1;

CREATE TABLE quote_submissions (
    submission_id TEXT PRIMARY KEY NOT NULL,
    wallet_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    intent_json TEXT NOT NULL,
    preflight_json TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('reserved','signed','submitted','unknown','failed','confirmed','finalized')),
    signature TEXT,
    signed_transaction BLOB,
    error TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    confirmation_rank INTEGER NOT NULL DEFAULT 0 CHECK (confirmation_rank BETWEEN 0 AND 3),
    observed_slot INTEGER,
    UNIQUE(wallet_id, idempotency_key),
    CHECK ((signature IS NULL) = (signed_transaction IS NULL)),
    CHECK (status NOT IN ('signed','submitted','confirmed','finalized') OR signature IS NOT NULL),
    CHECK ((confirmation_rank = 0 AND observed_slot IS NULL) OR
           (confirmation_rank > 0 AND observed_slot IS NOT NULL AND signature IS NOT NULL)),
    CHECK (status != 'confirmed' OR confirmation_rank = 2),
    CHECK (status != 'finalized' OR confirmation_rank = 3)
);

INSERT INTO quote_submissions(submission_id,wallet_id,idempotency_key,fingerprint,intent_json,preflight_json,status,signature,signed_transaction,error,created_at_ms,updated_at_ms)
SELECT submission_id,wallet_id,idempotency_key,fingerprint,intent_json,preflight_json,status,signature,signed_transaction,error,created_at_ms,updated_at_ms
FROM quote_submissions_v1;

DROP TABLE quote_submissions_v1;
