#!/usr/bin/env bash
set -euo pipefail
printf 'fake cargo:'
printf ' <%s>' "$@"
printf '\n'
case ${FAKE_CARGO_MODE:-success} in
    success) exit 0 ;;
    failure) exit 42 ;;
    wait) sleep 10 ;;
esac
