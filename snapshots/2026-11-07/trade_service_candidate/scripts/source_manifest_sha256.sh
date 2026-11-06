#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root_dir"

# Keep the file list explicit and sorted. AUDIT_MANIFEST.json and build output
# are intentionally excluded so the source fingerprint is reproducible.
{
  find src migrations docs scripts tests vendor/sol-trade-sdk -type f \
    ! -path '*/__pycache__/*' ! -path '*/target/*' ! -name '*.pyc' -print0
  printf '%s\0' Cargo.toml Cargo.lock trade-service-candidate.service trade-service-candidate-staging.service jupiter-shadow-staging.service
} | sort -z | xargs -0 sha256sum | sha256sum | awk '{print $1}'
