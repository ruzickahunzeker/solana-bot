#!/usr/bin/env python3
"""Read-only source identity check; never downloads, patches or executes the SDK."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import stat

ROOT = Path(__file__).resolve().parents[1]
LEDGER = ROOT / "docs/optimization/sdk-patches/6.0.2-local-delta.json"


def safe_path(name):
    if not isinstance(name, str):
        raise ValueError("source path must be a string")
    p = PurePosixPath(name)
    if not p.parts or p.is_absolute() or p.as_posix() != name or any(
            part in {"..", ".git"} for part in p.parts):
        raise ValueError("noncanonical source path: " + name)
    return name


def blob(data):
    return hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()


def file_entry(oid, mode):
    if not isinstance(oid, str) or not re.fullmatch(r"[a-f0-9]{40}", oid):
        raise ValueError("invalid Git blob identifier")
    if mode not in {"100644", "100755"}:
        raise ValueError("only regular source files are allowed")
    return oid, mode


def expected_files(tree, changes):
    if tree.get("truncated") is not False:
        raise ValueError("incomplete upstream inventory")
    expected = {}
    for entry in tree["tree"]:
        name = safe_path(entry["path"])
        if entry["type"] != "blob" or name in expected:
            raise ValueError("duplicate or non-file upstream entry")
        expected[name] = file_entry(entry["sha"], entry["mode"])
    changed = set()
    for entry in changes:
        name = safe_path(entry["path"])
        if name in changed:
            raise ValueError("duplicate local delta")
        changed.add(name)
        old = expected.get(name)
        if entry["before"] != (old[0] if old else None):
            raise ValueError("local delta does not match upstream: " + name)
        replacement = file_entry(entry["after"], entry["mode"])
        if replacement == old:
            raise ValueError("local delta is not a source change")
        expected[name] = replacement
    return expected


def check_tree(directory, expected):
    if directory.is_symlink() or not directory.is_dir():
        raise ValueError("source root must be a real directory")
    actual = {}
    for path in directory.rglob("*"):
        mode = path.lstat().st_mode
        if stat.S_ISLNK(mode) or not (stat.S_ISREG(mode) or stat.S_ISDIR(mode)):
            raise ValueError("source symlink or special file: " + str(path))
        if stat.S_ISREG(mode):
            name = safe_path(path.relative_to(directory).as_posix())
            actual[name] = file_entry(blob(path.read_bytes()),
                                      "100755" if mode & stat.S_IXUSR else "100644")
    if actual != expected:
        missing = sorted(expected.keys() - actual.keys())
        added = sorted(actual.keys() - expected.keys())
        different = sorted(k for k in expected.keys() & actual.keys() if expected[k] != actual[k])
        raise ValueError(f"source mismatch: missing={missing}, added={added}, changed={different}")
    return len(actual)


def checked_input(directory, name, digest):
    name = safe_path(name)
    path = directory / name
    if path.is_symlink() or any(p.is_symlink() for p in path.parents if p != directory.parent):
        raise ValueError("ledger input must not be a symlink")
    data = path.read_bytes()
    if hashlib.sha256(data).hexdigest() != digest:
        raise ValueError("ledger input checksum mismatch: " + name)
    return data


def validate(sdk_directory, upstream_directory=None):
    ledger = json.loads(LEDGER.read_text())
    if ledger["format_version"] != 1 or ledger["sdk_version"] != "6.0.2":
        raise ValueError("unsupported SDK source ledger")
    tree = json.loads(checked_input(LEDGER.parent, ledger["inventory"], ledger["inventory_sha256"]))
    checked_input(LEDGER.parent, ledger["patch"], ledger["patch_sha256"])
    if tree["sha"] != ledger["upstream_tree"]:
        raise ValueError("upstream tree identity mismatch")
    original = expected_files(tree, [])
    expected = expected_files(tree, ledger["files"])
    count = check_tree(sdk_directory, expected)
    if upstream_directory is not None:
        check_tree(upstream_directory, original)
    return {
        "sdk_version": ledger["sdk_version"], "upstream_commit": ledger["commit"],
        "signature_verification": ledger["signature_verification"],
        "upstream_files": len(original), "local_files": count,
        "modified": sum(e["before"] is not None for e in ledger["files"]),
        "added": sum(e["before"] is None for e in ledger["files"]),
        "upstream_directory_checked": upstream_directory is not None,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk-dir", type=Path, default=ROOT / "vendor/sol-trade-sdk")
    parser.add_argument("--upstream-dir", type=Path)
    args = parser.parse_args()
    try:
        print(json.dumps(validate(args.sdk_dir, args.upstream_dir), indent=2))
    except (ValueError, KeyError, TypeError, OSError) as error:
        parser.exit(1, f"SDK provenance check failed: {error}\n")


if __name__ == "__main__":
    main()
