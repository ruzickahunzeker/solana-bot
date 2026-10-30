CREATE TABLE IF NOT EXISTS alpha_shadow_orders (
  id TEXT PRIMARY KEY,
  signal_id TEXT NOT NULL UNIQUE,
  mint TEXT NOT NULL,
  side TEXT NOT NULL,
  pool TEXT NOT NULL,
  pool_id TEXT,
  quote_mint TEXT NOT NULL,
  amount_sol REAL,
  sell_percent INTEGER,
  status TEXT NOT NULL,
  idempotency_key TEXT NOT NULL UNIQUE,
  request_json TEXT NOT NULL,
  response_json TEXT,
  trade_service_request_id TEXT,
  error TEXT,
  created_at_ms INTEGER NOT NULL,
  updated_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_alpha_shadow_orders_reconcile
  ON alpha_shadow_orders(status, updated_at_ms);

CREATE INDEX IF NOT EXISTS idx_alpha_shadow_orders_request
  ON alpha_shadow_orders(trade_service_request_id);
