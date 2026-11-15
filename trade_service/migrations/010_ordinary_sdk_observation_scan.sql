CREATE INDEX idx_ordinary_sdk_observation_scan
ON ordinary_execution_claims(created_at_ms, request_id)
WHERE schema_version=1 AND operation='ordinary-sdk-trade'
  AND owner_nonce IS NOT NULL AND sdk_prepared_json IS NOT NULL
  AND unsigned_message IS NOT NULL AND unsigned_sha256 IS NOT NULL
  AND signed_signature IS NOT NULL AND signed_transaction IS NOT NULL
  AND signed_sha256 IS NOT NULL AND final_observation_json IS NULL
  AND submission_stage IN ('signed','broadcasting','submitted','unknown');
