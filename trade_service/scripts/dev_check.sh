#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
log_root=${TRADE_DEV_LOG_DIR:-"$root/target/dev-checks"}
mode=${1:-status}
mkdir -p -- "$log_root"

if [[ $mode == status ]]; then
    if [[ ! -f $log_root/latest ]]; then
        printf 'No recorded development check.\n'
        exit 0
    fi
    run=$(<"$log_root/latest")
    [[ $run =~ ^[0-9TZ-]+$ ]] || exit 2
    printf 'Run: %s\n' "$log_root/$run"
    cat -- "$log_root/$run/state"
    exec 9>"$log_root/check.lock"
    if flock -n 9 && [[ $(<"$log_root/$run/state") == running* ]]; then
        printf 'No runner holds the lock; the recorded result is unconfirmed, not still running.\n'
    fi
    if [[ -f $log_root/$run/boot_id && $(<"$log_root/$run/boot_id") != "$(</proc/sys/kernel/random/boot_id)" ]]; then
        printf 'Server rebooted since this run; a running marker is not a live process.\n'
    fi
    tail -n 15 -- "$log_root/$run/output.log"
    exit 0
fi

limit=${TRADE_DEV_TIMEOUT_SECONDS:-900}
heartbeat=${TRADE_DEV_HEARTBEAT_SECONDS:-15}
[[ $limit =~ ^[1-9][0-9]*$ && $heartbeat =~ ^[1-9][0-9]*$ ]] || {
    printf 'Timeout and heartbeat must be positive integer seconds.\n' >&2
    exit 2
}
cargo=${CARGO:-cargo}
case $mode in
    check) command=("$cargo" check --locked --offline --tests -j 1) ;;
    bin-check) command=("$cargo" check --locked --offline --bin pump-trade-service -j 1) ;;
    focused)
        filter=${2:-}
        [[ $filter =~ ^[A-Za-z0-9_:.-]+$ ]] || { printf 'Provide a Rust test filter.\n' >&2; exit 2; }
        command=("$cargo" test --locked --offline -j 1 "$filter" -- --test-threads=2)
        ;;
    full) command=("$cargo" test --locked --offline -j 1 -- --test-threads=2) ;;
    serial) command=("$cargo" test --locked --offline -j 1 -- --test-threads=1) ;;
    *) printf 'Usage: bash scripts/dev_check.sh {status|check|bin-check|focused FILTER|full|serial}\n' >&2; exit 2 ;;
esac

exec 9>"$log_root/check.lock"
flock -n 9 || { printf 'Another development check holds the lock; not starting a duplicate.\n' >&2; exit 75; }
run=$(date -u +%Y%m%dT%H%M%SZ)-$$
directory=$log_root/$run
mkdir -- "$directory"
printf '%s\n' "$run" >"$log_root/latest"
printf '%s\n' "$(</proc/sys/kernel/random/boot_id)" >"$directory/boot_id"
printf 'running mode=%s pid=%s\n' "$mode" "$$" >"$directory/state"
touch -- "$directory/output.log"
cd -- "$root"
printf 'Command:' >"$directory/command.txt"
printf ' %q' "${command[@]}" >>"$directory/command.txt"
printf '\n' >>"$directory/command.txt"
sha256sum Cargo.toml Cargo.lock >"$directory/manifests.sha256"
group=$(awk -F: '$1 == "0" {print $3}' /proc/self/cgroup)
events=/sys/fs/cgroup$group/memory.events
oom_before=0
if [[ $group == */solbot-dev-*.scope && -r $events ]]; then
    cp -- "$events" "$directory/memory.before"
    oom_before=$(awk '$1 == "oom_kill" {print $2}' "$events")
fi
printf 'Development check: %s\nLog: %s/output.log\n' "$mode" "$directory"
printf 'running mode=%s timeout=%ss\n' "$mode" "$limit" | tee -a "$directory/output.log"

started=$SECONDS
mkfifo -- "$directory/heartbeat.pipe"
exec 8<>"$directory/heartbeat.pipe"
(
    while ! IFS= read -r -t "$heartbeat" -u 8 _; do
        available=$(awk '/MemAvailable:/ {print $2}' /proc/meminfo)
        printf '[heartbeat] elapsed=%ss available_memory_kib=%s\n' "$((SECONDS - started))" "$available" | tee -a "$directory/output.log"
    done
) 9>&- &
beat=$!
cleanup() {
    printf 'stop\n' >&8 || true
    wait "$beat" 2>/dev/null || true
    exec 8>&-
}
trap cleanup EXIT
trap 'printf "interrupted\n" >"$directory/state"; exit 130' INT
trap 'printf "interrupted\n" >"$directory/state"; exit 143' TERM

set +e
timeout --signal=TERM --kill-after=15s "${limit}s" /usr/bin/time -v \
    nice -n 10 "${command[@]}" 2>&1 | tee -a "$directory/output.log"
statuses=("${PIPESTATUS[@]}")
set -e
result=${statuses[0]}
if [[ $result == 0 && ${statuses[1]} != 0 ]]; then result=${statuses[1]}; fi
oom_after=$oom_before
if [[ -f $directory/memory.before && -r $events ]]; then
    cp -- "$events" "$directory/memory.after"
    oom_after=$(awk '$1 == "oom_kill" {print $2}' "$events")
fi
if [[ $oom_after -gt $oom_before ]]; then
    state=resource_limited
    if [[ $result == 0 ]]; then result=137; fi
elif [[ $result == 0 ]]; then state=passed
elif [[ $result == 124 ]]; then state=timed_out
else state=failed
fi
printf '%s mode=%s exit=%s elapsed=%ss\n' "$state" "$mode" "$result" "$((SECONDS - started))" >"$directory/state"
cat -- "$directory/state"
exit "$result"
