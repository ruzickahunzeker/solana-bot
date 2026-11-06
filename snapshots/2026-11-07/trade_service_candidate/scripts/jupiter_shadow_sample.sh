#!/usr/bin/env bash
set -euo pipefail

# Read-only Jupiter quote sampler for an already running isolated candidate.
# It never builds, signs, submits, or stores provider response bodies.
umask 077

base_url="${JUPITER_SHADOW_BASE:-http://127.0.0.1:18887}"
token="${TRADE_SERVICE_LOCAL_API_TOKEN:-${LOCAL_TRADE_API_TOKEN:-}}"
interval="${JUPITER_SHADOW_INTERVAL_SECONDS:-60}"
duration="${JUPITER_SHADOW_DURATION_SECONDS:-86400}"
ready_timeout="${JUPITER_SHADOW_READY_TIMEOUT_SECONDS:-300}"
log_dir="${JUPITER_SHADOW_LOG_DIR:-/tmp/jupiter-shadow-runs}"
request_file="${JUPITER_SHADOW_REQUEST_FILE:-/tmp/jupiter-shadow-request.json}"

case "$interval" in (''|*[!0-9]*) echo "invalid JUPITER_SHADOW_INTERVAL_SECONDS" >&2; exit 2;; esac
case "$duration" in (''|*[!0-9]*) echo "invalid JUPITER_SHADOW_DURATION_SECONDS" >&2; exit 2;; esac
case "$ready_timeout" in (''|*[!0-9]*) echo "invalid JUPITER_SHADOW_READY_TIMEOUT_SECONDS" >&2; exit 2;; esac
[[ -n "$token" ]] || { echo "LOCAL_TRADE_API_TOKEN is required" >&2; exit 2; }

exec 9>/tmp/jupiter-shadow-sampler.lock
flock -n 9 || { echo "another Jupiter Shadow sampler is already running" >&2; exit 0; }

run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"
mkdir -p "$log_dir"
status_log="${JUPITER_SHADOW_STATUS_LOG:-$log_dir/run-$run_id.log}"
response_file="$(mktemp)"
trap 'rm -f "$request_file" "$response_file"' EXIT
printf '%s shadow_starting=true duration_seconds=%s interval_seconds=%s\n' \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$duration" "$interval" >"$status_log"

curl_args=(--silent --show-error --fail --max-time 30
  -H "Authorization: Bearer $token")
ready_deadline=$(( $(date +%s) + ready_timeout ))
while :; do
    if ready_json="$(curl "${curl_args[@]}" "$base_url/ready" 2>/dev/null)" && \
       wallet_id="$(python3 -c 'import json,sys; body=json.load(sys.stdin); value=body.get("default_wallet_id"); print(value) if isinstance(value,str) and value and all(body.get(key) is True for key in ("ready","rpc_ok","has_blockhash","trade_database_healthy")) else sys.exit(1)' <<<"$ready_json" 2>/dev/null)"; then
        break
    fi
    if (( $(date +%s) >= ready_deadline )); then
        printf '%s candidate_ready_timeout=true\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >>"$status_log"
        exit 1
    fi
    printf '%s waiting_for_candidate=true\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >>"$status_log"
    sleep 5
done
printf '%s shadow_started=true duration_seconds=%s interval_seconds=%s\n' \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$duration" "$interval" >>"$status_log"

cat >"$request_file" <<EOF
{"wallet_id":"$wallet_id","mint":"EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v","amount_lamports":1000000,"slippage_bps":100,"allowed_aggregators":["jupiter"]}
EOF
deadline=$(( $(date +%s) + duration ))
samples=0
while (( $(date +%s) < deadline )); do
    timestamp="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    : >"$response_file"
    if http_status="$(curl "${curl_args[@]}" -o "$response_file" -w '%{http_code}' \
        -X POST -H 'content-type: application/json' \
        "$base_url/v1/aggregators/quotes/compare" \
        --data-binary "@$request_file")"; then
        if result_status="$(python3 - "$response_file" 2>/dev/null <<'PY'
import json
import sys

body = json.load(open(sys.argv[1]))
if body.get("shadow_only") is not True:
    raise SystemExit("shadow_only marker missing")
providers = [item for item in body.get("results", []) if item.get("provider") == "jupiter"]
if len(providers) != 1:
    raise SystemExit("Jupiter result missing")
status = providers[0].get("status")
if status not in ("quoted", "failed", "unavailable"):
    raise SystemExit("invalid Jupiter result status")
print(status)
PY
        )"; then
            schema="ok"
        else
            schema="invalid"
            result_status="unknown"
        fi
        printf '%s http_status=%s schema=%s result_status=%s\n' \
            "$timestamp" "$http_status" "$schema" "$result_status" >>"$status_log"
    else
        printf '%s http_status=%s schema=unavailable\n' "$timestamp" "${http_status:-000}" >>"$status_log"
    fi
    samples=$((samples + 1))
    sleep "$interval"
done
printf '%s shadow_complete=true samples=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$samples" >>"$status_log"
