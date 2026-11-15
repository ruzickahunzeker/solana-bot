CREATE TABLE ordinary_execution_claims_operation_upgrade (
    request_id TEXT PRIMARY KEY NOT NULL REFERENCES trades(request_id) ON DELETE RESTRICT,
    schema_version INTEGER NOT NULL CHECK(schema_version = 1),
    operation TEXT NOT NULL CHECK(operation IN ('ordinary-sol-buy', 'ordinary-sdk-trade')),
    request_hash TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
    owner_nonce TEXT,
    submission_stage TEXT NOT NULL DEFAULT 'claimed',
    bound_quote_json TEXT,
    preflight_json TEXT,
    unsigned_message BLOB,
    unsigned_sha256 TEXT,
    signed_signature TEXT,
    signed_transaction BLOB,
    signed_sha256 TEXT,
    submission_error TEXT,
    final_observation_json TEXT,
    CHECK(operation = 'ordinary-sol-buy' OR (bound_quote_json IS NULL AND preflight_json IS NULL))
);

INSERT INTO ordinary_execution_claims_operation_upgrade (
    request_id, schema_version, operation, request_hash, created_at_ms,
    owner_nonce, submission_stage, bound_quote_json, preflight_json,
    unsigned_message, unsigned_sha256, signed_signature, signed_transaction,
    signed_sha256, submission_error, final_observation_json
)
SELECT request_id, schema_version, operation, request_hash, created_at_ms,
    owner_nonce, submission_stage, bound_quote_json, preflight_json,
    unsigned_message, unsigned_sha256, signed_signature, signed_transaction,
    signed_sha256, submission_error, final_observation_json
FROM ordinary_execution_claims;
