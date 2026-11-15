CREATE TABLE IF NOT EXISTS ordinary_execution_claims (
    request_id TEXT PRIMARY KEY NOT NULL REFERENCES trades(request_id) ON DELETE RESTRICT,
    schema_version INTEGER NOT NULL CHECK(schema_version = 1),
    operation TEXT NOT NULL CHECK(operation = 'ordinary-sol-buy'),
    request_hash TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0)
);
