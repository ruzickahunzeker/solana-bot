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
        step dependency-boundary-policy python3 scripts/test_dependency_boundaries.py
        step sdk-provenance-tests python3 scripts/test_sdk_provenance.py
        step sdk-provenance python3 scripts/check_sdk_provenance.py
        step locked-git-sources python3 scripts/test_ci_git_sources.py
        step namespace-probe python3 tests/fixtures/ci/namespace_probe.py
        step shell-runner bash scripts/test_dev_check.sh
        step sqlite-ddl python3 scripts/test_sdk_candidate_migration.py
        step sqlite-sdk-scan python3 scripts/test_sdk_observation_index.py
        step type-check cargo check --locked --offline --all-targets
        step boundary-inventory-tests command cargo test --manifest-path tools/type-boundary-inventory/Cargo.toml --target-dir target/boundary-tool --locked --offline
        step dependency-boundaries python3 scripts/check_dependency_boundaries.py
        step compile-tests cargo test --locked --offline --no-run
        step observation-migration cargo test --locked --offline ordinary_observation_005_schema_migrates_once_preserving_signed_rows_and_parent -- --test-threads=1
        step sdk-candidate cargo test --locked --offline sdk_candidate -- --test-threads=2
        step sdk-prepared-record cargo test --locked --offline sdk_prepared_record -- --test-threads=2
        step sdk-buy-evidence cargo test --locked --offline sdk_buy_ -- --test-threads=2
        step sdk-preparation cargo test --locked --offline ordinary_sdk_preparation -- --test-threads=2
        step preparation-presence cargo test --locked --offline preparation_presence -- --test-threads=2
        step parameter-session cargo test --locked --offline adapters::rpc::parameter_reads -- --test-threads=2
        step parameter-loaders cargo test --locked --offline protocol_params::tests::shared -- --test-threads=2
        step parameter-wire cargo test --locked --offline parameter_wire_ -- --test-threads=2
        step pool-discovery-session cargo test --locked --offline pool_discovery_session -- --test-threads=2
        step pool-discovery-loaders cargo test --locked --offline pool_discovery_loader -- --test-threads=2
        step pool-discovery-wire cargo test --locked --offline pool_discovery_wire -- --test-threads=2
        step sol-hop-session cargo test --locked --offline sol_hop_session -- --test-threads=2
        step sol-hop-loaders cargo test --locked --offline sol_hop_loader -- --test-threads=2
        step sol-hop-wire cargo test --locked --offline sol_hop_wire -- --test-threads=2
        step aggregator-account-runtime cargo test --locked --offline aggregator_account_read_ -- --test-threads=2
        step aggregator-account-loaders cargo test --locked --offline aggregator_account_loader_ -- --test-threads=2
        step aggregator-account-wire cargo test --locked --offline aggregator_account_wire_ -- --test-threads=2
        step aggregator-sim-runtime cargo test --locked --offline aggregator_sim_read_ -- --test-threads=2
        step aggregator-sim-loaders cargo test --locked --offline aggregator_sim_loader_ -- --test-threads=2
        step aggregator-sim-wire cargo test --locked --offline aggregator_sim_wire_ -- --test-threads=2
        step wsol-quote-runtime cargo test --locked --offline wsol_quote_read_ -- --test-threads=2
        step wsol-quote-loaders cargo test --locked --offline wsol_quote_loader_ -- --test-threads=2
        step wsol-quote-wire cargo test --locked --offline wsol_quote_wire_ -- --test-threads=2
        step wsol-status-shared cargo test --locked --offline wsol_status_shared_ -- --test-threads=2
        step wsol-status-wire cargo test --locked --offline wsol_status_wire_ -- --test-threads=2
        step wsol-status-contract cargo test --locked --offline wsol_status_ -- --test-threads=2
        step wsol-preparation-shared cargo test --locked --offline wsol_preparation_shared_ -- --test-threads=2
        step wsol-preparation-wire cargo test --locked --offline wsol_preparation_wire_ -- --test-threads=2
        step wsol-preparation-ownership cargo test --locked --offline wsol_preparation_ownership_ -- --test-threads=2
        step wsol-simulation-shared cargo test --locked --offline wsol_simulation_shared_ -- --test-threads=2
        step wsol-simulation-wire cargo test --locked --offline wsol_simulation_wire_ -- --test-threads=2
        step wsol-simulation-adapter cargo test --locked --offline wsol_simulation_adapter_ -- --test-threads=2
        step wsol-broadcast-lane cargo test --locked --offline wsol_broadcast_lane_ -- --test-threads=2
        step wsol-broadcast-adapter cargo test --locked --offline wsol_broadcast_adapter_ -- --test-threads=2
        step quote-status-shared cargo test --locked --offline quote_status_shared_ -- --test-threads=2
        step quote-status-wire cargo test --locked --offline quote_status_wire_ -- --test-threads=2
        step quote-status-adapter cargo test --locked --offline quote_status_adapter_ -- --test-threads=2
        step sdk-observer-shared cargo test --locked --offline sdk_observer_shared_ -- --test-threads=2
        step sdk-observer-wire cargo test --locked --offline sdk_observer_wire_ -- --test-threads=2
        step sdk-observer-flow cargo test --locked --offline shared_observer_flow_ -- --test-threads=2
        step quote-submit-status cargo test --locked --offline quote_submit_status_shared_ -- --test-threads=2
        step damm-candidate-shared cargo test --locked --offline damm_candidate_shared_ -- --test-threads=2
        step damm-candidate-adapter cargo test --locked --offline damm_candidate_adapter_ -- --test-threads=2
        step damm-candidate-wire cargo test --locked --offline damm_candidate_wire_ -- --test-threads=2
        step damm-quote-shared cargo test --locked --offline damm_quote_read_ -- --test-threads=2
        step damm-quote-adapter cargo test --locked --offline damm_quote_adapter_ -- --test-threads=2
        step damm-quote-wire cargo test --locked --offline damm_quote_wire_ -- --test-threads=2
        step sdk-recovery cargo test --locked --offline sdk_recovery -- --test-threads=2
        step sdk-scan cargo test --locked --offline sdk_scan -- --test-threads=2
        step sdk-signing-journal cargo test --locked --offline sdk_submission_journal -- --test-threads=2
        step sdk-submission-controller cargo test --locked --offline sdk_application_submission -- --test-threads=2
        step sdk-controlled-signing cargo test --locked --offline sdk_controlled_signing -- --test-threads=2
        step sdk-controlled-buy cargo test --locked --offline sdk_controlled_buy -- --test-threads=2
        step sdk-buy-claims cargo test --locked --offline sdk_buy_claim -- --test-threads=2
        step sdk-original-sender cargo test --locked --offline sdk_original_sender -- --test-threads=2
        step sdk-runtime-dispatch cargo test --locked --offline sdk_runtime_dispatch -- --test-threads=2
        step sdk-original-runtime cargo test --locked --offline sdk_original_runtime -- --test-threads=2
        step sdk-original-buy-runtime cargo test --locked --offline sdk_original_buy_runtime -- --test-threads=2
        step sdk-selected-observer cargo test --locked --offline sdk_selected_observer -- --test-threads=2
        step parameter-contract cargo test --locked --offline ordinary_param_contract_tests -- --test-threads=2
        step command-defaults cargo test --locked --offline command_defaults -- --test-threads=2
        step param-cache-port cargo test --locked --offline param_cache_port -- --test-threads=2
        step http-contract cargo test --locked --offline http_contract_tests -- --test-threads=2
        step api-access-policy cargo test --locked --offline api_access_policy -- --test-threads=2
        step validation-settings cargo test --locked --offline validation::settings_tests -- --test-threads=2
        step bootstrap-assembly cargo test --locked --offline bootstrap::tests -- --test-threads=2
        step readiness-settings cargo test --locked --offline readiness_settings -- --test-threads=2
        step ordinary-intake cargo test --locked --offline ordinary_intake -- --test-threads=2
        step ordinary-runtime cargo test --locked --offline ordinary_runtime -- --test-threads=2
        step backend-contracts cargo test --locked --offline backend_contract -- --test-threads=2
        step trade-task-lifecycle cargo test --locked --offline trade_tasks -- --test-threads=2
        step vanity-lifecycle cargo test --locked --offline vanity_lifecycle_ -- --test-threads=2
        step vanity-shutdown cargo test --locked --offline vanity_shutdown_ -- --test-threads=2
        step service-shutdown cargo test --locked --offline service_shutdown -- --test-threads=2
        step task-drain-settings cargo test --locked --offline trade_task_drain_configuration -- --test-threads=2
        step shadow-drain cargo test --locked --offline shadow_drain -- --test-threads=2
        step shadow-shutdown cargo test --locked --offline shadow_shutdown -- --test-threads=2
        step damm-buy-probe cargo test --locked --offline damm_buy_ -- --test-threads=2
        step damm-output-evidence cargo test --locked --offline damm_output_evidence -- --test-threads=2
        step damm-preview-context cargo test --locked --offline damm_unsigned_simulation -- --test-threads=2
        step damm-compiled-binding cargo test --locked --offline damm_compiled_binding -- --test-threads=2
        step damm-candidate cargo test --locked --offline damm_sell_message -- --test-threads=2
        step damm-probe cargo test --locked --offline damm_probe -- --test-threads=2
        step sdk-lanes cargo test --locked --offline ordinary_lanes -- --test-threads=2
        step full-concurrent cargo test --locked --offline -- --test-threads=2
        step full-serial cargo test --locked --offline -- --test-threads=1
        git diff --exit-code -- Cargo.toml Cargo.lock vendor src tools
        ;;
    *) printf 'Usage: ci_validate.sh {preflight|tests}\n' >&2; exit 2 ;;
esac
