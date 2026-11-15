#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
temporary=$(mktemp -d)
trap 'rm -rf -- "$temporary"' EXIT
export TRADE_DEV_LOG_DIR=$temporary/logs
export TRADE_DEV_HEARTBEAT_SECONDS=1
export CARGO=$temporary/cargo
cp -- "$root/tests/fixtures/dev_check/fake_cargo.sh" "$CARGO"
chmod +x -- "$CARGO"
runner=$root/scripts/dev_check.sh

bash "$runner" check >/dev/null
run=$(<"$TRADE_DEV_LOG_DIR/latest")
[[ $(<"$TRADE_DEV_LOG_DIR/$run/state") == passed* ]]
rg -q -- 'check.*--locked.*--offline.*--tests.*-j.*1' "$TRADE_DEV_LOG_DIR/$run/command.txt"
bash "$runner" bin-check >/dev/null
run=$(<"$TRADE_DEV_LOG_DIR/latest")
[[ $(<"$TRADE_DEV_LOG_DIR/$run/state") == passed* ]]
rg -q -- '--bin pump-trade-service' "$TRADE_DEV_LOG_DIR/$run/command.txt"

set +e
FAKE_CARGO_MODE=failure bash "$runner" focused sdk_candidate >/dev/null
code=$?
set -e
[[ $code == 42 ]]
run=$(<"$TRADE_DEV_LOG_DIR/latest")
[[ $(<"$TRADE_DEV_LOG_DIR/$run/state") == failed*exit=42* ]]

set +e
TRADE_DEV_TIMEOUT_SECONDS=1 FAKE_CARGO_MODE=wait bash "$runner" check >/dev/null
code=$?
set -e
[[ $code == 124 ]]
run=$(<"$TRADE_DEV_LOG_DIR/latest")
[[ $(<"$TRADE_DEV_LOG_DIR/$run/state") == timed_out* ]]

exec 8>"$TRADE_DEV_LOG_DIR/check.lock"
flock -n 8
set +e
bash "$runner" check >/dev/null 2>&1
code=$?
set -e
[[ $code == 75 ]]
flock -u 8
bash "$runner" status >/dev/null
printf 'running mode=check pid=0\n' >"$TRADE_DEV_LOG_DIR/$run/state"
status=$(bash "$runner" status)
[[ $status == *'result is unconfirmed'* ]]
set +e
TRADE_DEV_TIMEOUT_SECONDS=0 bash "$runner" check >/dev/null 2>&1
code=$?
set -e
[[ $code == 2 ]]
printf 'dev_check: success, failure, timeout, duplicate-lock and status tests passed\n'
