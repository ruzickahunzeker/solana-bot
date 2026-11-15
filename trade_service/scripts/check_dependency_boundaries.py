#!/usr/bin/env python3
"""Compiler-backed production type boundaries; CI mutations are always restored."""
from contextlib import contextmanager
import json
import os
from pathlib import Path, PurePosixPath
import re
import signal
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
POLICY = ROOT / "tests/fixtures/contracts/dependency_boundaries_v1.json"
GUARD = "#![cfg_attr(not(test), forbid(clippy::disallowed_types))]"


def read_policy(path=POLICY):
    policy = json.loads(path.read_text(encoding="utf-8"))
    if set(policy) != {"schema_version", "guarded_modules", "forbidden_types"} or type(policy["schema_version"]) is not int or policy["schema_version"] != 1:
        raise ValueError("Unknown dependency boundary policy")
    modules, types = policy["guarded_modules"], policy["forbidden_types"]
    if not modules or not types:
        raise ValueError("Empty boundary policy")
    seen = set()
    for module in modules:
        p = PurePosixPath(module["path"])
        if set(module) != {"name", "path"} or p.parts != ("src", module["name"], "mod.rs") or not re.fullmatch(r"[a-z_]+", module["name"]):
            raise ValueError("Invalid guarded module")
        if module["name"] in seen:
            raise ValueError("Duplicate guarded module")
        seen.add(module["name"])
    seen = set()
    for item in types:
        if not {"id", "path"} <= set(item) <= {"id", "path", "probe"}:
            raise ValueError("Unknown type entry")
        if not re.fullmatch(r"[a-z][a-z_0-9]*", item["id"]) or item["id"] in seen:
            raise ValueError("Invalid or duplicate type id")
        seen.add(item["id"])
        for value in (item["path"], item.get("probe", item["path"])):
            if not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]*(?:::[A-Za-z_][A-Za-z_0-9]*)+", value):
                raise ValueError("Type paths cannot contain generated Rust or configuration")
    return policy


def config_text(policy):
    entries = ["{ path = " + json.dumps(item["path"]) +
               ', reason = "Use project-owned types and ports in this layer" }'
               for item in policy["forbidden_types"]]
    return "disallowed-types = [\n  " + ",\n  ".join(entries) + "\n]\n"


def include_aliases(policy, aliases):
    policy = json.loads(json.dumps(policy))
    known = {item["path"] for item in policy["forbidden_types"]}
    for index, path in enumerate(aliases):
        if not re.fullmatch(r"pump_trade_service(?:::[A-Za-z_][A-Za-z_0-9]*)+", path):
            raise ValueError("Invalid compiler alias inventory")
        if path not in known:
            policy["forbidden_types"].append({"id": f"inventory_alias_{index}", "path": path})
            known.add(path)
    return policy


def probe_source(original, policy):
    lines = ["", "mod dependency_boundary_negative_probe {",
             "    use sol_trade_sdk::TradingClient as HiddenSdkClient;",
             "    pub struct Leaks {"]
    expected = {}
    prefix_lines = len(original.decode("utf-8").splitlines())
    for item in policy["forbidden_types"]:
        path = "HiddenSdkClient" if item["id"] == "sdk_client" else item.get("probe", item["path"])
        lines.append(f'        pub {item["id"]}: Option<{path}>,')
        expected[item["id"]] = prefix_lines + len(lines)
    lines += ["    }", "}"]
    return original + ("\n".join(lines) + "\n").encode("utf-8"), expected


def verify_negative(messages, expected):
    hits = set()
    for entry in messages:
        if entry.get("reason") != "compiler-message":
            continue
        diagnostic = entry["message"]
        if diagnostic.get("level") != "error":
            continue
        code = (diagnostic.get("code") or {}).get("code")
        spans = diagnostic.get("spans", [])
        if code is None and not spans and diagnostic.get("message", "").startswith("aborting due to "):
            continue
        if code != "clippy::disallowed_types":
            raise ValueError("Negative probe failed for a non-boundary compiler error")
        for span in spans:
            if span.get("is_primary"):
                for key, line in expected.get(span["file_name"], {}).items():
                    if span["line_start"] <= line <= span["line_end"]:
                        hits.add((span["file_name"], key))
    required = {(path, key) for path, fields in expected.items() for key in fields}
    if hits != required:
        raise ValueError("Missing compiler rejections: " + repr(sorted(required - hits)))
    return len(hits)


@contextmanager
def mutated_modules(root, policy, alias_probe=False):
    originals, expected = {}, {}
    try:
        if alias_probe:
            path = root / "src/main.rs"
            if path.is_symlink() or not path.is_file():
                raise ValueError("Alias probe root must be a regular source file")
            originals[path] = path.read_bytes()
            path.write_bytes(originals[path] + b"\nmod dependency_boundary_alias_source { pub type Hidden = sol_trade_sdk::TradingClient; }\n")
        for module in policy["guarded_modules"]:
            path = root / module["path"]
            if path.is_symlink() or not path.is_file():
                raise ValueError("Guarded source must be a regular file")
            original = path.read_bytes()
            if not original.decode("utf-8").startswith(GUARD + "\n") or not original.endswith(b"\n"):
                raise ValueError("Missing non-test compiler guard or source newline")
            originals[path] = original
            source, fields = probe_source(original, policy)
            path.write_bytes(source)
            expected[module["path"]] = fields
        yield expected
    finally:
        for path, original in originals.items():
            path.write_bytes(original)
        if any(path.read_bytes() != original for path, original in originals.items()):
            raise RuntimeError("Source restoration failed")


def require_isolated_ci(root):
    if not all(os.environ.get(key) == value for key, value in {
        "GITHUB_ACTIONS": "true", "RUNNER_ENVIRONMENT": "github-hosted", "TRADE_CI_NETWORK_ISOLATED": "1"
    }.items()):
        raise RuntimeError("Only the source-only isolated GitHub job may mutate probes")
    if any((root / name).exists() for name in (".env", ".wallets.passphrase", "data", "wallets.json", "wallets.json.enc")):
        raise RuntimeError("Runtime inputs are forbidden")
    links = subprocess.check_output(["ip", "-o", "link", "show"], text=True).splitlines()
    routes = subprocess.check_output(["ip", "route", "show", "default"], text=True).strip()
    if len(links) != 1 or routes:
        raise RuntimeError("Only loopback networking is allowed")


def compiler_check(root, config, label):
    logs = root / "target/ci-validation/logs"
    logs.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, CLIPPY_CONF_DIR=str(config))
    command = ["cargo", "--config", str(root / "ci/locked-git-sources.toml"), "clippy", "--locked", "--offline",
               "--bin", "pump-trade-service", "--message-format=json", "--", "-A", "clippy::all"]
    with subprocess.Popen(command, cwd=root, env=env, text=True, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, start_new_session=True) as process:
        try:
            stdout, stderr = process.communicate(timeout=600)
        except BaseException:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
            raise
    (logs / f"dependency-boundaries-{label}.log").write_text(stdout + stderr, encoding="utf-8")
    messages = [json.loads(line) for line in stdout.splitlines() if line.strip()]
    print(f"dependency-boundaries {label}: exit={process.returncode}", flush=True)
    return process.returncode, messages


def collect_aliases(root, policy):
    inventory = subprocess.check_output([
        "cargo", "run", "--manifest-path", str(root / "tools/type-boundary-inventory/Cargo.toml"),
        "--target-dir", str(root / "target/boundary-tool"), "--locked", "--offline", "--quiet", "--",
        str(root), *[module["name"] for module in policy["guarded_modules"]],
    ], text=True, timeout=120)
    aliases = json.loads(inventory)
    print(f"Parsed {len(aliases)} outside-layer Rust type aliases", flush=True)
    return aliases


def main():
    require_isolated_ci(ROOT)
    def interrupt(_signum, _frame):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, interrupt)
    print(subprocess.check_output(["clippy-driver", "--version"], text=True).strip(), flush=True)
    policy = read_policy()
    for module in policy["guarded_modules"]:
        if not (ROOT / module["path"]).read_text().startswith(GUARD + "\n"):
            raise RuntimeError("The production module guard is missing")
    output = ROOT / "target/ci-validation"
    output.mkdir(parents=True, exist_ok=True)
    aliases = collect_aliases(ROOT, policy)
    config_policy = include_aliases(policy, aliases)
    with tempfile.TemporaryDirectory(prefix="boundary-config-", dir=output) as directory:
        config = Path(directory)
        (config / "clippy.toml").write_text(config_text(config_policy), encoding="utf-8")
        code, _ = compiler_check(ROOT, config, "original")
        if code != 0:
            raise RuntimeError("Production dependency boundary failed; inspect original log")
        probe_policy = json.loads(json.dumps(policy))
        probe_policy["forbidden_types"].append({
            "id": "outside_alias", "path": "pump_trade_service::dependency_boundary_alias_source::Hidden",
            "probe": "crate::dependency_boundary_alias_source::Hidden",
        })
        with mutated_modules(ROOT, probe_policy, alias_probe=True) as expected:
            negative_config = include_aliases(policy, collect_aliases(ROOT, policy))
            (config / "clippy.toml").write_text(config_text(negative_config), encoding="utf-8")
            code, messages = compiler_check(ROOT, config, "negative")
            if code == 0:
                raise RuntimeError("Forbidden types unexpectedly compiled")
            count = verify_negative(messages, expected)
        (config / "clippy.toml").write_text(config_text(config_policy), encoding="utf-8")
        code, _ = compiler_check(ROOT, config, "restored")
        if code != 0:
            raise RuntimeError("Restored production source failed")
        print(f"Production boundary accepted; {count} scoped negative fields rejected; source restored", flush=True)


if __name__ == "__main__":
    main()
