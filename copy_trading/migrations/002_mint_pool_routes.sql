CREATE TABLE IF NOT EXISTS mint_pool_routes (
  mint TEXT PRIMARY KEY,
  pool TEXT NOT NULL,
  pool_id TEXT,
  quote_mint TEXT NOT NULL,
  source TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
