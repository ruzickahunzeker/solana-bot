#!/usr/bin/env bash
set -euo pipefail
[[ ${GITHUB_ACTIONS:-} == true && ${RUNNER_ENVIRONMENT:-} == github-hosted ]] || {
    printf 'Only the isolated GitHub-hosted validation job may enter this path.\n' >&2
    exit 2
}
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
exec sudo --non-interactive unshare --net -- /bin/bash -c '
    set -euo pipefail
    ip link set lo up
    uid=$1; gid=$2; home=$3; root=$4; toolchain=$5
    [[ $(ip -o link show | wc -l) == 1 ]] || exit 2
    cd -- "$root"
    exec setpriv --reuid "$uid" --regid "$gid" --init-groups \
        env -i HOME="$home" USER=runner LANG=C.UTF-8 TZ=UTC \
        PATH="$home/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" \
        CARGO_HOME="$home/.cargo" RUSTUP_HOME="$home/.rustup" \
        RUSTUP_TOOLCHAIN="$toolchain" CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 \
        CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_TERM_COLOR=never \
        CI=true GITHUB_ACTIONS=true RUNNER_ENVIRONMENT=github-hosted \
        TRADE_CI_NETWORK_ISOLATED=1 bash scripts/ci_validate.sh tests
' ci-test-namespace "$(id -u)" "$(id -g)" "$HOME" "$root" "${RUST_TOOLCHAIN:?}"
