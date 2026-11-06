#!/usr/bin/env python3
"""Verify either the frozen staging baseline or an isolated candidate artifact."""

import hashlib
import argparse
import json
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument(
    "--staging-baseline",
    action="store_true",
    help="verify the frozen 18887 binary without comparing its source to current source",
)
parser.add_argument(
    "--candidate-artifact",
    type=Path,
    help="directory containing an isolated pump-trade-service and BUILD_INFO",
)
args = parser.parse_args()
if args.staging_baseline == (args.candidate_artifact is not None):
    parser.error("choose exactly one of --staging-baseline or --candidate-artifact")


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def require_equal(label: str, actual: str, recorded: str) -> None:
    if actual != recorded:
        raise SystemExit(f"audit manifest mismatch: {label}")


def read_build_info(path: Path) -> dict[str, str]:
    return dict(
        line.split("=", 1)
        for line in path.read_text().splitlines()
        if "=" in line
    )


source_hash = subprocess.check_output(
    [str(ROOT / "scripts/source_manifest_sha256.sh")], text=True
).strip()
if args.candidate_artifact is not None:
    artifact = args.candidate_artifact.resolve(strict=True)
    info = read_build_info(artifact / "BUILD_INFO")
    binary_hash = digest(artifact / "pump-trade-service")
    require_equal("isolated binary SHA-256", binary_hash, info["sha256"])
    require_equal("isolated source SHA-256", source_hash, info["source_manifest_sha256"])
    require_equal("isolated Cargo.toml", digest(ROOT / "Cargo.toml"), info["cargo_toml_sha256"])
    require_equal("isolated Cargo.lock", digest(ROOT / "Cargo.lock"), info["cargo_lock_sha256"])
    print(f"candidate artifact passed: binary={binary_hash} source={source_hash}")
    raise SystemExit(0)

MANIFEST = json.loads((ROOT / "AUDIT_MANIFEST.json").read_text())
BUILD_INFO = read_build_info(ROOT / "BUILD_INFO")


binary = ROOT / "target/release/pump-trade-service"
require_equal("binary path", str(binary), MANIFEST["binary"]["path"])
binary_hash = digest(binary)
require_equal("binary SHA-256", binary_hash, MANIFEST["binary"]["sha256"])
require_equal("BUILD_INFO binary SHA-256", binary_hash, BUILD_INFO["sha256"])
for name, field in (
    ("Cargo.toml", "cargo_toml_sha256"),
    ("Cargo.lock", "cargo_lock_sha256"),
    ("trade-service-candidate.service", "candidate_unit_sha256"),
):
    require_equal(name, digest(ROOT / name), MANIFEST[field])
require_equal("source root", str(ROOT), MANIFEST["source_root"])
require_equal(
    "frozen staging source SHA-256",
    BUILD_INFO["source_manifest_sha256"],
    MANIFEST["source_manifest_sha256"],
)
current = (ROOT.parent / "current").resolve(strict=True)
require_equal("current target", str(current), MANIFEST["paths"]["current"])
env_values = {}
for line in (ROOT / ".env").read_text().splitlines():
    if "=" in line and not line.lstrip().startswith("#"):
        key, value = line.split("=", 1)
        env_values[key.strip()] = value.strip()
for key, expected in MANIFEST["runtime_guards"]["env_verified"].items():
    require_equal(key, env_values.get(key, ""), expected)
print(
    "audit manifest passed (frozen staging baseline): "
    f"binary={binary_hash} recorded_source={BUILD_INFO['source_manifest_sha256']} "
    f"current_source={source_hash}"
)
