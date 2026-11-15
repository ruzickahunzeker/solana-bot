#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd -- "$root"
output=$root/target/ci-validation
mkdir -p -- "$output/logs"

[[ ${GITHUB_ACTIONS:-} == true && ${RUNNER_ENVIRONMENT:-} == github-hosted ]] || {
    printf 'Do not run this large validation pipeline on the trading VM.\n' >&2
    exit 2
}

step() {
    local label=$1
    shift
    local started=$SECONDS result
    printf '\n== %s ==\n' "$label"
    set +e
    "$@" 2>&1 | tee "$output/logs/$label.log"
    local statuses=("${PIPESTATUS[@]}")
    set -e
    result=${statuses[0]}
    if [[ $result == 0 && ${statuses[1]} != 0 ]]; then result=${statuses[1]}; fi
    printf '%s\t%s\t%s\n' "$label" "$result" "$((SECONDS - started))" >>"$output/results.tsv"
    return "$result"
}

cargo() {
    command cargo --config "$root/ci/locked-git-sources.toml" "$@"
}

case ${1:-} in
    preflight)
        [[ ! -e .env && ! -e .wallets.passphrase && ! -e data && ! -e wallets.json && ! -e wallets.json.enc ]] || {
            printf 'Runtime configuration or wallet data is present; refusing CI.\n' >&2
            exit 2
        }
        available=$(awk '/MemAvailable:/ {print $2}' /proc/meminfo)
        [[ $available -ge 6000000 ]] || { printf 'Insufficient runner memory: %s KiB\n' "$available" >&2; exit 2; }
        disk=$(df -Pk . | awk 'NR==2 {print $4}')
        [[ $disk -ge 12000000 ]] || { printf 'Insufficient runner disk: %s KiB\n' "$disk" >&2; exit 2; }
        {
            git rev-parse HEAD
            printf 'rust_toolchain=%s\n' "${RUST_TOOLCHAIN:?}"
            printf 'available_memory_kib=%s\navailable_disk_kib=%s\n' "$available" "$disk"
            sha256sum Cargo.toml Cargo.lock vendor/sol-trade-sdk/Cargo.toml
        } >"$output/identity.txt"
        python3 - <<'PY'
from pathlib import PurePosixPath
import subprocess
paths = subprocess.check_output(["git", "ls-files", "-z"]).decode().split("\0")
bad = []
for name in filter(None, paths):
    p = PurePosixPath(name)
    if (p.name in {".env", ".wallets.passphrase", "wallets.json", "wallets.json.enc"}
            or p.suffix in {".sqlite", ".db", ".key", ".pem", ".enc"}
            or any(part in {"data", "backups", "logs", "target"} for part in p.parts)):
        bad.append(name)
if bad:
    raise SystemExit("Tracked runtime/secret paths are forbidden: " + ", ".join(bad))
print("Source-only path preflight passed")
PY
        ;;
    tests)
        [[ ${TRADE_CI_NETWORK_ISOLATED:-} == 1 ]] || { printf 'Network isolation is required.\n' >&2; exit 2; }
        [[ $(ip -o link show | wc -l) == 1 && -z $(ip route show default) ]] || exit 2
        exec 9>"$output/validation.lock"
        flock -n 9 || exit 75
        printf 'step\texit_code\telapsed_seconds\n' >"$output/results.tsv"
        step formatting cargo fmt --check
        step workflow-policy python3 scripts/test_ci_workflow.py
        step sdk-provenance-tests python3 scripts/test_sdk_provenance.py
        step sdk-provenance python3 scripts/check_sdk_provenance.py
        step locked-git-sources python3 scripts/test_ci_git_sources.py
        step namespace-probe python3 tests/fixtures/ci/namespace_probe.py
        step shell-runner bash scripts/test_dev_check.sh
        step sqlite-ddl python3 scripts/test_sdk_candidate_migration.py
        step type-check cargo check --locked --offline --all-targets
        step compile-tests cargo test --locked --offline --no-run
        step sdk-candidate cargo test --locked --offline sdk_candidate -- --test-threads=2
        step sdk-preparation cargo test --locked --offline ordinary_sdk_preparation -- --test-threads=2
        step parameter-contract cargo test --locked --offline ordinary_param_contract_tests -- --test-threads=2
        step http-contract cargo test --locked --offline http_contract_tests -- --test-threads=2
        step damm-output-evidence cargo test --locked --offline damm_output_evidence -- --test-threads=2
        step damm-preview-context cargo test --locked --offline damm_unsigned_simulation -- --test-threads=2
        step damm-candidate cargo test --locked --offline damm_sell_message -- --test-threads=2
        step damm-probe cargo test --locked --offline damm_probe -- --test-threads=2
        step sdk-lanes cargo test --locked --offline ordinary_lanes -- --test-threads=2
        step full-concurrent cargo test --locked --offline -- --test-threads=2
        step full-serial cargo test --locked --offline -- --test-threads=1
        git diff --exit-code -- Cargo.toml Cargo.lock vendor
        ;;
    *) printf 'Usage: ci_validate.sh {preflight|tests}\n' >&2; exit 2 ;;
esac
