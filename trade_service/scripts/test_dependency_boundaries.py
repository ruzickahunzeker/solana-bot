#!/usr/bin/env python3
"""Policy/restore regressions and one tiny metadata-only compiler alias probe."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("boundaries", ROOT / "scripts/check_dependency_boundaries.py")
boundaries = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(boundaries)


def message(path, line, code="clippy::disallowed_types"):
    return {"reason": "compiler-message", "message": {"level": "error", "code": {"code": code},
            "spans": [{"file_name": path, "line_start": line, "line_end": line, "is_primary": True}]}}


class DependencyBoundaryTests(unittest.TestCase):
    def test_current_policy_covers_three_layers_and_selected_external_and_runtime_types(self):
        policy = boundaries.read_policy()
        self.assertEqual({module["name"] for module in policy["guarded_modules"]}, {"application", "domain", "protocol"})
        types = {item["id"] for item in policy["forbidden_types"]}
        self.assertTrue({"keypair", "sdk_client", "dex_params", "rpc", "sqlite_pool", "http_headers", "app_state", "config", "trade_index", "blockhash_cache", "trade_tasks", "dispatch_ticket", "damm_buy_quote", "damm_buy_message", "damm_buy_candidate", "damm_buy_signed", "parameter_session", "discovery_session", "sol_hop_session", "aggregator_simulation_plan", "aggregator_simulation_session", "message", "wsol_quote_session"} <= types)
        for module in policy["guarded_modules"]:
            self.assertTrue((ROOT / module["path"]).read_text().startswith(boundaries.GUARD + "\n"))
        self.assertIn("Use project-owned types and ports", boundaries.config_text(policy))
        self.assertIn("wsol_status_reader", types)
        self.assertTrue({"wsol_unsigned", "wsol_prepared"} <= types)
        self.assertNotIn("allow-imports", boundaries.config_text(policy))

    def test_bad_schema_duplicate_names_and_generated_code_are_rejected(self):
        original = boundaries.read_policy()
        candidates = []
        for version in [2, True]:
            changed = copy.deepcopy(original)
            changed["schema_version"] = version
            candidates.append(changed)
        for key in ["guarded_modules", "forbidden_types"]:
            changed = copy.deepcopy(original)
            changed[key].append(changed[key][0])
            candidates.append(changed)
        changed = copy.deepcopy(original)
        changed["guarded_modules"][0]["path"] = "../src/application/mod.rs"
        candidates.append(changed)
        changed = copy.deepcopy(original)
        changed["forbidden_types"][0]["probe"] = 'crate::Type>; fn bypass() {}'
        candidates.append(changed)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "policy.json"
            for candidate in candidates:
                path.write_text(json.dumps(candidate))
                with self.assertRaises(ValueError):
                    boundaries.read_policy(path)

    def test_probe_lines_bind_each_type_to_its_own_scope_including_alias(self):
        policy = boundaries.read_policy()
        source, expected = boundaries.probe_source((boundaries.GUARD + "\n").encode(), policy)
        lines = source.decode().splitlines()
        self.assertEqual(len(expected), len(policy["forbidden_types"]))
        for key, line in expected.items():
            self.assertIn(f"pub {key}: Option<", lines[line - 1])
        self.assertIn("HiddenSdkClient", lines[expected["sdk_client"] - 1])

    def test_missing_rejection_or_unrelated_compiler_error_cannot_pass(self):
        expected = {"src/application/mod.rs": {"sdk": 3}, "src/domain/mod.rs": {"sdk": 4}}
        complete = [message("src/application/mod.rs", 3), message("src/domain/mod.rs", 4)]
        self.assertEqual(boundaries.verify_negative(complete, expected), 2)
        for messages in [[], complete[:1], complete + [message("src/domain/mod.rs", 4, "E0433")]]:
            with self.assertRaises(ValueError):
                boundaries.verify_negative(messages, expected)

    def test_borrowed_session_probe_uses_only_the_fixed_static_lifetime(self):
        policy = boundaries.read_policy()
        source, expected = boundaries.probe_source((boundaries.GUARD + "\n").encode(), policy)
        line = source.decode().splitlines()[expected["aggregator_simulation_session"] - 1]
        self.assertIn("Option<crate::adapters::rpc::aggregator_simulation::AggregatorSimulationReadSession<'static>>", line)
        self.assertNotIn("<'static>", source.decode().splitlines()[expected["aggregator_simulation_plan"] - 1])
        self.assertIn("WsolQuoteReadSession<'static>>", source.decode().splitlines()[expected["wsol_quote_session"] - 1])

    def test_static_lifetime_metadata_cannot_inject_rust_or_replace_path_validation(self):
        policy = boundaries.read_policy()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "policy.json"
            for value in [False, 1, None, "'a", "static>; fn bypass() {}"]:
                changed = copy.deepcopy(policy)
                changed["forbidden_types"][-1]["static_lifetime"] = value
                path.write_text(json.dumps(changed))
                with self.assertRaises(ValueError):
                    boundaries.read_policy(path)
            changed = copy.deepcopy(policy)
            changed["forbidden_types"][-1]["probe"] += "<'static>"
            path.write_text(json.dumps(changed))
            with self.assertRaises(ValueError):
                boundaries.read_policy(path)

    def test_sources_are_restored_byte_for_byte_even_when_validation_raises(self):
        policy = boundaries.read_policy()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            originals = {}
            for module in policy["guarded_modules"]:
                path = root / module["path"]
                path.parent.mkdir(parents=True)
                originals[path] = (boundaries.GUARD + "\n\n// preserved original bytes\n").encode()
                path.write_bytes(originals[path])
            with self.assertRaisesRegex(RuntimeError, "synthetic failure"):
                with boundaries.mutated_modules(root, policy) as expected:
                    self.assertEqual(len(expected), 3)
                    self.assertTrue(all(path.read_bytes() != old for path, old in originals.items()))
                    raise RuntimeError("synthetic failure")
            self.assertTrue(all(path.read_bytes() == old for path, old in originals.items()))

    def test_non_ci_run_is_rejected_before_source_changes_or_subprocesses(self):
        with mock.patch.dict(os.environ, {}, clear=True), mock.patch.object(boundaries.subprocess, "check_output") as command:
            with self.assertRaises(RuntimeError):
                boundaries.require_isolated_ci(ROOT)
            command.assert_not_called()

    def test_alias_probe_root_and_partial_module_setup_are_restored_on_failure(self):
        policy = boundaries.read_policy()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "src").mkdir()
            main = root / "src/main.rs"
            original = b"fn main() {}\n"
            main.write_bytes(original)
            module = root / policy["guarded_modules"][0]["path"]
            module.parent.mkdir()
            module.write_text(boundaries.GUARD + "\n")
            with self.assertRaises(ValueError):
                with boundaries.mutated_modules(root, policy, alias_probe=True):
                    self.fail("incomplete module setup must not be accepted")
            self.assertEqual(main.read_bytes(), original)
            self.assertEqual(module.read_text(), boundaries.GUARD + "\n")

    def test_runtime_inputs_are_rejected_even_with_forged_ci_environment(self):
        env = {"GITHUB_ACTIONS": "true", "RUNNER_ENVIRONMENT": "github-hosted", "TRADE_CI_NETWORK_ISOLATED": "1"}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / ".env").touch()
            with mock.patch.dict(os.environ, env, clear=True), mock.patch.object(boundaries.subprocess, "check_output") as command:
                with self.assertRaises(RuntimeError):
                    boundaries.require_isolated_ci(root)
                command.assert_not_called()

    def test_tiny_real_compiler_rejects_local_resource_through_direct_and_reexported_aliases(self):
        policy = boundaries.read_policy()
        policy["forbidden_types"] = [item for item in policy["forbidden_types"] if item["id"] == "config"]
        policy = boundaries.include_aliases(policy, ["pump_trade_service::adapter::WrappedConfig"])
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "clippy.toml").write_text(boundaries.config_text(policy))
            result = subprocess.run([
                "clippy-driver", "--crate-name", "pump_trade_service", "--edition", "2021", "--emit=metadata",
                "--error-format=json", "--out-dir", str(root),
                str(ROOT / "tests/fixtures/contracts/dependency_boundary_alias_probe.rs"),
                "-A", "clippy::all",
            ], env=dict(os.environ, CLIPPY_CONF_DIR=str(root)), text=True, capture_output=True, timeout=30)
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            errors = [json.loads(line) for line in result.stderr.splitlines() if line.startswith("{")]
            fields = {12, 13, 14}
            observed = set()
            for error in errors:
                if (error.get("code") or {}).get("code") == "clippy::disallowed_types":
                    observed.update(span["line_start"] for span in error["spans"] if span.get("is_primary"))
            self.assertTrue(fields <= observed, result.stderr)

    def test_cargo_clippy_forwards_the_locked_configuration_to_its_cargo_check_child(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "src").mkdir()
            (root / "ci").mkdir()
            (root / "Cargo.toml").write_text('[package]\nname="pump-trade-service"\nversion="0.0.0"\nedition="2021"\n')
            (root / "src/main.rs").write_text("fn main() {}\n")
            (root / "ci/locked-git-sources.toml").write_text('[build]\ntarget="boundary-invalid-target"\n')
            subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=root, check=True, capture_output=True, timeout=30)
            command = boundaries.cargo_clippy_command(root)
            self.assertLess(command.index("clippy"), command.index("--config"))
            result = subprocess.run(command, cwd=root, text=True, capture_output=True, timeout=30)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("boundary-invalid-target", result.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
