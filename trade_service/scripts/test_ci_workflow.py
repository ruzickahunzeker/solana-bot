#!/usr/bin/env python3
"""Static workflow policy checks; the namespace probe separately tests isolation."""
from pathlib import Path
import re
import unittest
import yaml

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/trade-service-validation.yml"


class WorkflowPolicyTests(unittest.TestCase):
    def setUp(self):
        self.text = WORKFLOW.read_text(encoding="utf-8")
        self.workflow = yaml.load(self.text, Loader=yaml.BaseLoader)
        self.job = self.workflow["jobs"]["validate"]
        self.steps = self.job["steps"]

    def test_only_validation_branches_trigger_push_and_permissions_are_read_only(self):
        self.assertEqual(self.workflow["on"]["push"]["branches"], ["ci/trade-service-validation-*"])
        self.assertNotIn("pull_request_target", self.workflow["on"])
        self.assertEqual(self.workflow["permissions"], {"contents": "read"})
        self.assertEqual(self.workflow["concurrency"]["cancel-in-progress"], "false")

    def test_official_actions_are_pinned_and_checkout_drops_credentials(self):
        actions = [step for step in self.steps if "uses" in step]
        self.assertEqual(len(actions), 2)
        for step in actions:
            self.assertRegex(step["uses"], r"^actions/(checkout|upload-artifact)@[a-f0-9]{40}$")
        self.assertEqual(actions[0]["with"]["persist-credentials"], "false")
        self.assertNotIn("secrets.", self.text)

    def test_artifacts_are_logs_and_identity_only(self):
        upload = next(step for step in self.steps if step.get("uses", "").startswith("actions/upload-artifact@"))
        self.assertEqual(upload["if"], "always()")
        self.assertEqual(set(upload["with"]["path"].splitlines()), {
            "trade_service/target/ci-validation/logs/",
            "trade_service/target/ci-validation/identity.txt",
            "trade_service/target/ci-validation/results.tsv",
        })

    def test_no_release_or_deploy_and_network_isolation_is_mandatory(self):
        commands = "\n".join(step.get("run", "") for step in self.steps)
        self.assertNotRegex(commands, r"cargo\s+build|systemctl|scp\s|ssh\s")
        self.assertIn("cargo --config ci/locked-git-sources.toml fetch --locked", commands)
        self.assertIn("bash scripts/ci_test_namespace.sh", commands)
        wrapper = (ROOT / "trade_service/scripts/ci_test_namespace.sh").read_text()
        self.assertIn("unshare --net", wrapper)
        self.assertIn("env -i", wrapper)
        self.assertIn("setpriv --reuid", wrapper)

    def test_complete_suites_are_not_replaced_by_focused_tests(self):
        script = (ROOT / "trade_service/scripts/ci_validate.sh").read_text()
        self.assertIn("step full-concurrent cargo test --locked --offline -- --test-threads=2", script)
        self.assertIn("step full-serial cargo test --locked --offline -- --test-threads=1", script)
        self.assertIn("step sdk-preparation", script)
        self.assertIn("step sdk-signing-journal", script)
        self.assertIn("step sdk-submission-controller", script)
        self.assertIn("step sdk-controlled-signing", script)
        self.assertIn("step sdk-original-sender", script)
        self.assertIn("step sdk-runtime-dispatch", script)
        self.assertIn("step sdk-original-runtime cargo test --locked --offline sdk_original_runtime -- --test-threads=2", script)
        self.assertIn("step observation-migration cargo test --locked --offline ordinary_observation_005_schema_migrates_once_preserving_signed_rows_and_parent -- --test-threads=1", script)
        self.assertIn("step sdk-selected-observer", script)
        self.assertIn("step sdk-recovery", script)
        self.assertIn("step command-defaults cargo test --locked --offline command_defaults -- --test-threads=2", script)
        self.assertIn("step param-cache-port cargo test --locked --offline param_cache_port -- --test-threads=2", script)
        self.assertIn("step api-access-policy cargo test --locked --offline api_access_policy -- --test-threads=2", script)
        self.assertIn("step validation-settings cargo test --locked --offline validation::settings_tests -- --test-threads=2", script)
        self.assertIn("step bootstrap-assembly cargo test --locked --offline bootstrap::tests -- --test-threads=2", script)
        self.assertIn("step readiness-settings cargo test --locked --offline readiness_settings -- --test-threads=2", script)
        self.assertIn("step sdk-scan", script)
        self.assertIn("step sqlite-sdk-scan python3 scripts/test_sdk_observation_index.py", script)
        self.assertIn("step damm-output-evidence", script)
        self.assertIn("step damm-preview-context", script)
        self.assertIn("step damm-compiled-binding", script)
        self.assertNotIn("--ignored", script)
        self.assertNotIn("cargo build --release", script)

    def test_shell_runner_tools_are_installed_explicitly(self):
        commands = "\n".join(step.get("run", "") for step in self.steps)
        install = next(line for line in commands.splitlines() if "apt-get install" in line)
        self.assertIn("ripgrep", install.split())
        self.assertIn("time", install.split())
        self.assertIn("python3-yaml", install.split())

    def test_sdk_source_identity_and_negative_tests_precede_rust_compilation(self):
        script = (ROOT / "trade_service/scripts/ci_validate.sh").read_text()
        for command in ["step sdk-provenance-tests python3 scripts/test_sdk_provenance.py",
                        "step sdk-provenance python3 scripts/check_sdk_provenance.py"]:
            self.assertLess(script.index(command), script.index("step type-check"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
