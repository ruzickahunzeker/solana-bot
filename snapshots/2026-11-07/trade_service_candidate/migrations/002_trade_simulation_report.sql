-- Add the structured simulation report to databases created before v1.
-- The migration runner checks the existing schema first because v1 candidate
-- databases already declared this column in the initial schema.
ALTER TABLE trades ADD COLUMN simulation_json TEXT;
