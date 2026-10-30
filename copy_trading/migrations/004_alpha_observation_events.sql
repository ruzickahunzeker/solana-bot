CREATE TABLE IF NOT EXISTS alpha_observation_events (
  event_id TEXT PRIMARY KEY,
  signature TEXT,
  mint TEXT,
  pool TEXT,
  pool_id TEXT,
  quote_mint TEXT,
  tx_type TEXT,
  signer TEXT,
  event_timestamp_ms INTEGER,
  received_unix_ms INTEGER NOT NULL,
  route_status TEXT NOT NULL,
  raw_json TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_alpha_observation_events_mint_time
  ON alpha_observation_events(mint, received_unix_ms DESC);

CREATE INDEX IF NOT EXISTS idx_alpha_observation_events_signature
  ON alpha_observation_events(signature);
