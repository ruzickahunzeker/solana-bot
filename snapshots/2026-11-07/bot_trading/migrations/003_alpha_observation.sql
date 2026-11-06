CREATE TABLE IF NOT EXISTS alpha_observation_events (
  event_id TEXT PRIMARY KEY,
  signature TEXT,
  mint TEXT,
  pool TEXT,
  pool_id TEXT,
  quote_mint TEXT,
  tx_type TEXT,
  signer TEXT,
  raw_json TEXT NOT NULL,
  received_at_ms INTEGER NOT NULL,
  inserted_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_alpha_observation_mint_time
  ON alpha_observation_events(mint, received_at_ms);

CREATE TABLE IF NOT EXISTS alpha_paper_signals (
  signal_id TEXT PRIMARY KEY,
  event_id TEXT NOT NULL,
  mint TEXT NOT NULL,
  pool TEXT,
  pool_id TEXT,
  quote_mint TEXT,
  side TEXT NOT NULL,
  score REAL NOT NULL,
  status TEXT NOT NULL DEFAULT 'paper',
  features_json TEXT NOT NULL,
  created_at_ms INTEGER NOT NULL,
  inserted_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_alpha_paper_signals_mint_time
  ON alpha_paper_signals(mint, created_at_ms);
