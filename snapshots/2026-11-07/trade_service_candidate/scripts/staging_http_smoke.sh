#!/usr/bin/env bash
set -euo pipefail

# Read-only smoke check for an already running, isolated candidate instance.
# This script never starts a service, signs a transaction, or calls a submit route.
base_url="${CANDIDATE_STAGING_URL:-http://127.0.0.1:18887}"
token="${TRADE_SERVICE_LOCAL_API_TOKEN:-${LOCAL_TRADE_API_TOKEN:-}}"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

curl_args=(--silent --show-error --fail --max-time "${SMOKE_TIMEOUT_SECONDS:-10}")
if [[ -n "$token" ]]; then
    curl_args+=( -H "Authorization: Bearer $token" )
fi

echo "[1/4] health"
curl "${curl_args[@]}" "$base_url/health" >"$tmp_dir/health.json"
python3 - "$tmp_dir/health.json" <<'PY'
import json, sys
body = json.load(open(sys.argv[1]))
if body.get("ok") is not True:
    raise SystemExit("health response did not report ok=true")
PY

echo "[2/5] ready"
curl "${curl_args[@]}" "$base_url/ready" >"$tmp_dir/ready.json"
python3 - "$tmp_dir/ready.json" <<'PY'
import json, sys
body = json.load(open(sys.argv[1]))
for key in ("ready", "rpc_ok", "has_blockhash"):
    if not isinstance(body.get(key), bool):
        raise SystemExit(f"ready response missing boolean field: {key}")
PY

echo "[3/5] capabilities"
curl "${curl_args[@]}" "$base_url/v1/aggregators/capabilities" >"$tmp_dir/capabilities.json"
python3 - "$tmp_dir/capabilities.json" <<'PY'
import json, sys
body = json.load(open(sys.argv[1]))
if body.get("simulate_only") is not True:
    raise SystemExit("candidate is not simulate_only")
providers = {item.get("provider"): item for item in body.get("providers", [])}
jupiter = providers.get("jupiter")
dflow = providers.get("dflow")
if not jupiter or jupiter.get("supports_quote") is not True:
    raise SystemExit("Jupiter quote capability is missing")
for key in ("supports_build", "supports_preflight", "supports_submit"):
    if jupiter.get(key) is not False:
        raise SystemExit(f"Jupiter {key} must remain disabled before Gate 2")
if not dflow or dflow.get("read_only") is not True or dflow.get("supports_quote") is not False:
    raise SystemExit("DFlow must be disabled/read-only")
print("simulate_only=true jupiter=quote-only dflow=disabled-read-only")
PY

echo "[4/6] DFlow unavailable compare"
python3 - "$tmp_dir/ready.json" "$tmp_dir/dflow-request.json" <<'PY'
import json, sys
ready = json.load(open(sys.argv[1]))
wallet_id = ready.get("default_wallet_id")
if not isinstance(wallet_id, str) or not wallet_id:
    raise SystemExit("ready response did not include default_wallet_id")
json.dump({
    "wallet_id": wallet_id,
    "mint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
    "amount_lamports": 1000000,
    "slippage_bps": 100,
    "allowed_aggregators": ["dflow"],
}, open(sys.argv[2], "w"))
PY
curl "${curl_args[@]}" -X POST "$base_url/v1/aggregators/quotes/compare" \
    -H 'content-type: application/json' --data-binary "@$tmp_dir/dflow-request.json" \
    >"$tmp_dir/dflow-compare.json"
python3 - "$tmp_dir/dflow-compare.json" <<'PY'
import json, sys
body = json.load(open(sys.argv[1]))
if body.get("shadow_only") is not True:
    raise SystemExit("compare response was not shadow_only")
results = [item for item in body.get("results", []) if item.get("provider") == "dflow"]
if len(results) != 1 or results[0].get("status") != "unavailable":
    raise SystemExit("DFlow compare did not return unavailable")
PY

echo "[5/6] shadow metrics"
curl "${curl_args[@]}" "$base_url/v1/aggregators/shadow/metrics?provider=dflow&limit=1" >"$tmp_dir/shadow.json"
python3 - "$tmp_dir/shadow.json" <<'PY'
import json, sys
body = json.load(open(sys.argv[1]))
if body.get("read_only") is not True:
    raise SystemExit("shadow endpoint did not report read_only=true")
if not isinstance(body.get("metrics"), list) or not isinstance(body.get("summary"), dict):
    raise SystemExit("shadow response shape is invalid")
if body["summary"].get("unavailable_count", 0) < 1:
    raise SystemExit("DFlow unavailable metric was not persisted")
PY

echo "[6/6] no-submit invariant"
echo "smoke performs diagnostics and one DFlow read-only compare; no build, sign, or submit request is sent"

echo "staging HTTP smoke passed: $base_url"
