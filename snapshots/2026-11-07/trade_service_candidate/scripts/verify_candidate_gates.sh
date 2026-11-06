#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root_dir"

toolchain_bin="/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin"
export PATH="${toolchain_bin}:/usr/bin:/bin"
export CARGO_TARGET_DIR="${CANDIDATE_GATE_TARGET_DIR:-/tmp/candidate-sdk-route-target}"
export CARGO_BUILD_JOBS=1
if [[ "$(readlink -f "$CARGO_TARGET_DIR")" == "$(readlink -f "$root_dir/target")" ]]; then
    echo "gate target must be isolated from the 18887 runtime binary" >&2
    exit 2
fi

required_cargo="1.99.0"
actual_cargo="$(cargo --version | awk '{print $2}')"
if [[ "$(printf '%s\n' "$required_cargo" "$actual_cargo" | sort -V | head -n1)" != "$required_cargo" ]]; then
    echo "toolchain guard failed: Cargo $actual_cargo is older than required Cargo $required_cargo" >&2
    echo "install or select the pinned compatible toolchain before running candidate gates" >&2
    exit 2
fi

echo "[1/7] format"
cargo fmt --all -- --check
source_before="$(scripts/source_manifest_sha256.sh)"
echo "[2/7] locked check"
cargo check --locked
echo "[3/7] tests"
cargo test --locked
echo "[4/7] release build"
cargo build --release --locked
echo "[5/7] runtime guard"
assert_env_value() {
    local file="$1" key="$2" expected="$3"
    local value
    value="$(awk -F= -v key="$key" '$1 == key { value=$0; sub(/^[^=]*=/, "", value); print value }' "$file" | tail -n 1)"
    test "$value" = "$expected"
}
assert_env_value .env TRADE_SERVICE_SIMULATE_ONLY true
if grep -Eq '^SDK_TWO_HOP_LIVE_ENABLED=(true|1|yes|on)$' .env; then
    echo "candidate .env must not enable SDK two-hop live submission" >&2
    exit 1
fi
assert_env_value .env CONVERSION_LIVE_ENABLED false
assert_env_value .env ENABLE_SIMULATE true
grep -Fxq 'Environment=TRADE_SERVICE_SIMULATE_ONLY=true' trade-service-candidate.service
grep -Fq 'SDK_TWO_HOP_LIVE_ENABLED=false' trade-service-candidate-staging.service
if grep -Eq '(/v1/aggregators/quotes/submit|sign_transaction|build_transaction)' scripts/staging_http_smoke.sh; then
    echo "staging smoke guard failed: script contains a build/sign/submit path" >&2
    exit 1
fi
grep -Fq '/v1/aggregators/quotes/compare' scripts/staging_http_smoke.sh
python3 -m py_compile scripts/verify_staging_persistence.py
python3 -m py_compile scripts/audit_jupiter_shadow_progress.py
python3 -m py_compile scripts/verify_jupiter_shadow.py
python3 -m py_compile scripts/verify_route_decision_http.py
python3 -m py_compile scripts/verify_funded_sdk_preview.py
python3 -m py_compile scripts/verify_cpmm_sell_preview_http.py
python3 -m py_compile scripts/verify_cpmm_sell_order_http.py
python3 -m py_compile scripts/verify_cpmm_buy_order_http.py
python3 -m py_compile scripts/probe_pump_usdc_curve.py
PYTHONPATH=scripts python3 -m unittest -q \
    scripts/test_audit_jupiter_shadow_progress.py \
    scripts/test_verify_jupiter_shadow.py
bash -n scripts/staging_http_smoke.sh
if [[ -f /root/solana/bot_trading/scripts/verify_simulate_only_config.py ]]; then
    python3 /root/solana/bot_trading/scripts/verify_simulate_only_config.py
    if [[ -f /root/solana/bot_trading/bot-trading.service ]]; then
        grep -Fq 'ExecStartPre=/usr/bin/python3 /root/solana/bot_trading/scripts/verify_simulate_only_config.py' \
            /root/solana/bot_trading/bot-trading.service
    fi
fi
echo "[6/7] artifact hashes"
actual_source="$(scripts/source_manifest_sha256.sh)"
test "$actual_source" = "$source_before"
actual_binary="$(sha256sum "$CARGO_TARGET_DIR/release/pump-trade-service" | awk '{print $1}')"
artifact_dir="/root/solana/trade_service/builds/candidate-${actual_binary:0:12}-${actual_source:0:12}"
if [[ ! -e "$artifact_dir" ]]; then
    artifact_temp="$(mktemp -d /root/solana/trade_service/builds/.candidate-gate-XXXXXXXX)"
    trap 'rm -rf "$artifact_temp"' EXIT
    cp "$CARGO_TARGET_DIR/release/pump-trade-service" "$artifact_temp/pump-trade-service"
    cat >"$artifact_temp/BUILD_INFO" <<EOF
role=candidate
base_backend=sdk11
sdk_version=11.0.1
status=isolated-gates-passed
live_enabled=false
toolchain=cargo-$(cargo --version | awk '{print $2}'),rustc-$(rustc --version | awk '{print $2}')
source_manifest_sha256=$actual_source
cargo_toml_sha256=$(sha256sum Cargo.toml | awk '{print $1}')
cargo_lock_sha256=$(sha256sum Cargo.lock | awk '{print $1}')
sha256=$actual_binary
EOF
    mv "$artifact_temp" "$artifact_dir"
    trap - EXIT
fi
python3 scripts/verify_audit_manifest.py --candidate-artifact "$artifact_dir"
python3 scripts/verify_audit_manifest.py --staging-baseline
echo "[7/7] current remains unchanged"
test "$(readlink -f /root/solana/trade_service/current)" = "/root/solana/trade_service/releases/sdk-6.0.2-legacy"
test "$(systemctl is-active trade-service-candidate.service 2>/dev/null || true)" != active
if command -v ss >/dev/null 2>&1; then
    if socket_list="$(ss -ltnH 2>/dev/null)"; then
        ! awk '$4 ~ /:18788$/ { found = 1 } END { exit found ? 0 : 1 }' <<<"$socket_list"
        echo "18788_listener_check=passed"
    else
        echo "18788_listener_check=unavailable (netlink access denied)" >&2
    fi
fi
printf 'candidate gates passed: binary=%s source=%s artifact=%s\n' "$actual_binary" "$actual_source" "$artifact_dir"
