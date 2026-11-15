CREATE TABLE IF NOT EXISTS wallet_sol_transfer_attempts (
  transfer_id TEXT PRIMARY KEY REFERENCES wallet_sol_transfers(transfer_id),
  fingerprint TEXT NOT NULL CHECK(length(fingerprint)=64),
  intent_json TEXT NOT NULL,
  state TEXT NOT NULL CHECK(state IN ('reserved','signed','submitted','confirmed','unknown','failed')),
  snapshot_json TEXT,
  signature TEXT,
  signed_transaction BLOB,
  error TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CHECK (
    (snapshot_json IS NULL AND signature IS NULL AND signed_transaction IS NULL
      AND state IN ('reserved','failed','unknown'))
    OR
    (snapshot_json IS NOT NULL AND signature IS NOT NULL AND length(signature)>0
      AND signed_transaction IS NOT NULL AND length(signed_transaction)>0
      AND state IN ('signed','submitted','confirmed','unknown'))
  )
);
