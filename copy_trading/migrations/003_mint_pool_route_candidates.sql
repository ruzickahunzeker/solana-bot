CREATE TABLE IF NOT EXISTS mint_pool_route_candidates (
  route_key TEXT PRIMARY KEY,
  mint TEXT NOT NULL,
  pool TEXT NOT NULL,
  pool_id TEXT NOT NULL DEFAULT '',
  quote_mint TEXT NOT NULL,
  source TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_mint_pool_route_candidates_mint
  ON mint_pool_route_candidates (mint, updated_at);

INSERT OR IGNORE INTO mint_pool_route_candidates
  (route_key, mint, pool, pool_id, quote_mint, source, updated_at)
SELECT
  mint || '|' || pool || '|' || COALESCE(pool_id, '') || '|' || quote_mint,
  mint,
  pool,
  COALESCE(pool_id, ''),
  quote_mint,
  source,
  updated_at
FROM mint_pool_routes;
