PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS app_users (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  telegram_id INTEGER NOT NULL UNIQUE,
  username TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS wallets (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id INTEGER NOT NULL REFERENCES app_users(id),
  external_wallet_id TEXT,
  label TEXT NOT NULL,
  public_key TEXT NOT NULL,
  is_active INTEGER NOT NULL DEFAULT 0,
  is_deleted INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(user_id, public_key)
);

CREATE TABLE IF NOT EXISTS wallet_secrets (
  wallet_id INTEGER PRIMARY KEY REFERENCES wallets(id),
  encrypted_private_key TEXT NOT NULL,
  nonce TEXT NOT NULL,
  key_version INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS watch_wallets (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id INTEGER NOT NULL REFERENCES app_users(id),
  address TEXT NOT NULL,
  label TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  is_deleted INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(user_id, address)
);

CREATE TABLE IF NOT EXISTS strategies (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id INTEGER NOT NULL REFERENCES app_users(id),
  wallet_id INTEGER REFERENCES wallets(id),
  watch_wallet_id INTEGER REFERENCES watch_wallets(id),
  name TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 0,
  dry_run INTEGER NOT NULL DEFAULT 1,
  execution_mode TEXT NOT NULL DEFAULT 'dry_run',
  execution_method TEXT NOT NULL DEFAULT 'sdk',
  copy_buy INTEGER NOT NULL DEFAULT 1,
  copy_sell INTEGER NOT NULL DEFAULT 0,
  buy_amount_sol REAL NOT NULL,
  buy_mode TEXT NOT NULL DEFAULT 'fixed',
  buy_ratio_percent REAL NOT NULL DEFAULT 100,
  sell_percent REAL,
  max_slippage_bps INTEGER NOT NULL,
  tip_sol REAL NOT NULL,
  max_per_mint_sol REAL NOT NULL DEFAULT 0.05,
  daily_max_sol REAL NOT NULL DEFAULT 0.2,
  cooldown_secs INTEGER NOT NULL DEFAULT 60,
  only_once INTEGER NOT NULL DEFAULT 0,
  min_sol_in_pool REAL NOT NULL DEFAULT 0,
  min_mcap REAL NOT NULL DEFAULT 0,
  max_mcap REAL NOT NULL DEFAULT 0,
  min_target_buy_sol REAL NOT NULL DEFAULT 0,
  max_target_buy_sol REAL NOT NULL DEFAULT 0,
  max_signal_delay_ms INTEGER,
  allowed_pools TEXT,
  mev_mode TEXT NOT NULL DEFAULT 'none',
  live INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS pumpapi_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  signature TEXT,
  mint TEXT,
  pool TEXT,
  pool_id TEXT,
  quote_mint TEXT,
  tx_type TEXT,
  signer TEXT,
  raw_json TEXT NOT NULL,
  received_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(signature, tx_type) ON CONFLICT IGNORE
);

CREATE TABLE IF NOT EXISTS signals (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id INTEGER REFERENCES app_users(id),
  strategy_id INTEGER REFERENCES strategies(id),
  source_event_id INTEGER REFERENCES pumpapi_events(id),
  mint TEXT NOT NULL,
  side TEXT NOT NULL,
  reason TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'created',
  metadata_json TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS orders (
  id TEXT PRIMARY KEY,
  user_id INTEGER NOT NULL REFERENCES app_users(id),
  wallet_id INTEGER REFERENCES wallets(id),
  strategy_id INTEGER REFERENCES strategies(id),
  signal_id INTEGER REFERENCES signals(id),
  mint TEXT NOT NULL,
  side TEXT NOT NULL,
  amount_sol REAL,
  sell_percent REAL,
  status TEXT NOT NULL,
  idempotency_key TEXT NOT NULL UNIQUE,
  trade_service_request_json TEXT,
  trade_service_response_json TEXT,
  tx_signature TEXT,
  error TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS order_attempts (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  order_id TEXT NOT NULL REFERENCES orders(id),
  status TEXT NOT NULL,
  request_json TEXT,
  response_json TEXT,
  error TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS dry_run_positions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id INTEGER NOT NULL REFERENCES app_users(id),
  strategy_id INTEGER NOT NULL REFERENCES strategies(id),
  wallet_id INTEGER REFERENCES wallets(id),
  mint TEXT NOT NULL,
  amount_sol REAL NOT NULL DEFAULT 0,
  buy_count INTEGER NOT NULL DEFAULT 0,
  sell_count INTEGER NOT NULL DEFAULT 0,
  last_buy_signal_id INTEGER REFERENCES signals(id),
  last_sell_signal_id INTEGER REFERENCES signals(id),
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(user_id, strategy_id, mint)
);

CREATE TABLE IF NOT EXISTS limit_orders (
  id TEXT PRIMARY KEY,
  user_id INTEGER NOT NULL REFERENCES app_users(id),
  wallet_id INTEGER REFERENCES wallets(id),
  mint TEXT NOT NULL,
  side TEXT NOT NULL,
  trigger_type TEXT NOT NULL,
  limit_action TEXT NOT NULL DEFAULT '',
  trigger_metric TEXT NOT NULL DEFAULT 'price_usd',
  trigger_price REAL NOT NULL,
  trigger_currency TEXT NOT NULL DEFAULT 'USD',
  amount_sol REAL,
  sell_percent REAL,
  dry_run INTEGER NOT NULL DEFAULT 1,
  execution_mode TEXT NOT NULL DEFAULT 'dry_run',
  execution_method TEXT NOT NULL DEFAULT 'sdk',
  slippage_bps INTEGER NOT NULL DEFAULT 1500,
  tip_sol REAL NOT NULL DEFAULT 0.0001,
  mev_mode TEXT NOT NULL DEFAULT 'none',
  status TEXT NOT NULL DEFAULT 'pending',
  triggered_price REAL,
  triggered_currency TEXT,
  triggered_source_event_id INTEGER REFERENCES pumpapi_events(id),
  triggered_at TEXT,
  expires_at TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS risk_blocks (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id INTEGER REFERENCES app_users(id),
  strategy_id INTEGER REFERENCES strategies(id),
  mint TEXT,
  reason TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS audit_logs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id INTEGER REFERENCES app_users(id),
  action TEXT NOT NULL,
  target_type TEXT,
  target_id TEXT,
  metadata_json TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS system_state (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS user_trade_settings (
  user_id INTEGER PRIMARY KEY REFERENCES app_users(id),
  use_mev INTEGER NOT NULL DEFAULT 0,
  default_mev_provider TEXT NOT NULL DEFAULT 'auto',
  default_execution_mode TEXT NOT NULL DEFAULT 'dry_run',
  default_execution_method TEXT NOT NULL DEFAULT 'sdk',
  default_slippage_bps INTEGER NOT NULL DEFAULT 1500,
  default_tip_sol REAL NOT NULL DEFAULT 0.0001,
  default_buy_preset_slot INTEGER NOT NULL DEFAULT 3,
  default_sell_preset_slot INTEGER NOT NULL DEFAULT 5,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS user_trade_presets (
  user_id INTEGER NOT NULL REFERENCES app_users(id),
  side TEXT NOT NULL,
  slot INTEGER NOT NULL,
  value REAL NOT NULL,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (user_id, side, slot)
);
