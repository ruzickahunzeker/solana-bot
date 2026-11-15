CREATE INDEX IF NOT EXISTS idx_ordinary_signed_observation_scan
ON ordinary_execution_claims(created_at_ms, request_id)
WHERE schema_version=1 AND operation='ordinary-sol-buy'
  AND owner_nonce IS NOT NULL AND signed_signature IS NOT NULL
  AND signed_transaction IS NOT NULL AND final_observation_json IS NULL
  AND submission_stage IN ('signed','submitted','unknown');
