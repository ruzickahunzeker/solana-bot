CREATE TABLE IF NOT EXISTS alpha_paper_signals (
  signal_id TEXT PRIMARY KEY,
  event_id TEXT NOT NULL,
  mint TEXT NOT NULL,
  pool TEXT,
  pool_id TEXT,
  side TEXT NOT NULL,
  score REAL NOT NULL,
  status TEXT NOT NULL DEFAULT 'paper',
  features_json TEXT NOT NULL,
  created_at_ms INTEGER NOT NULL,
  UNIQUE(event_id, side)
);

CREATE INDEX IF NOT EXISTS idx_alpha_paper_signals_mint_time
  ON alpha_paper_signals(mint, created_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_alpha_paper_signals_status_time
  ON alpha_paper_signals(status, created_at_ms DESC);
