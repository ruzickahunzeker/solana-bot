#!/usr/bin/env python3
"""Public temporary-file tests for the read-only SDK source checker."""
from pathlib import Path
import tempfile
import unittest

from check_sdk_provenance import blob, checked_input, check_tree, expected_files, safe_path


class SdkProvenanceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "src").mkdir()
        self.file = self.root / "src/lib.rs"
        self.file.write_bytes(b"pub fn public_fixture() {}\n")
        self.file.chmod(0o644)
        self.original = blob(self.file.read_bytes())
        self.expected = {"src/lib.rs": (self.original, "100644")}
        self.tree = {"truncated": False, "tree": [
            {"path": "src/lib.rs", "sha": self.original, "mode": "100644", "type": "blob"}
        ]}

    def test_exact_contents_and_mode_pass_without_writing(self):
        self.assertEqual(check_tree(self.root, self.expected), 1)
        self.assertEqual(self.file.read_bytes(), b"pub fn public_fixture() {}\n")

    def test_modified_missing_and_extra_files_are_rejected(self):
        self.file.write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "changed=.*src/lib.rs"):
            check_tree(self.root, self.expected)
        self.file.unlink()
        with self.assertRaisesRegex(ValueError, "missing=.*src/lib.rs"):
            check_tree(self.root, self.expected)
        self.file.write_bytes(b"pub fn public_fixture() {}\n")
        (self.root / "unexpected.rs").write_bytes(b"extra")
        with self.assertRaisesRegex(ValueError, "added=.*unexpected.rs"):
            check_tree(self.root, self.expected)

    def test_executable_bit_changes_are_not_ignored(self):
        self.file.chmod(0o755)
        with self.assertRaisesRegex(ValueError, "source mismatch"):
            check_tree(self.root, self.expected)

    def test_symlink_roots_files_and_directories_are_rejected(self):
        link = self.root / "root-link"
        link.symlink_to(self.root, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "real directory"):
            check_tree(link, self.expected)
        with self.assertRaisesRegex(ValueError, "symlink"):
            check_tree(self.root, self.expected)
        link.unlink()
        self.file.unlink()
        self.file.symlink_to(self.root / "absent")
        with self.assertRaisesRegex(ValueError, "symlink"):
            check_tree(self.root, self.expected)

    def test_noncanonical_and_git_metadata_paths_are_rejected(self):
        for name in ["", ".", "/src.rs", "../src.rs", "src/../x", "src//x", "./src.rs", ".git/config"]:
            with self.subTest(name=name), self.assertRaises(ValueError):
                safe_path(name)
        self.assertEqual(safe_path(".github/workflows/build.yml"), ".github/workflows/build.yml")

    def test_delta_binds_original_and_expected_added_files(self):
        replacement = blob(b"reviewed change")
        changes = [
            {"path": "src/lib.rs", "before": self.original, "after": replacement, "mode": "100644"},
            {"path": "new.rs", "before": None, "after": replacement, "mode": "100644"},
        ]
        self.assertEqual(expected_files(self.tree, changes), {
            "src/lib.rs": (replacement, "100644"), "new.rs": (replacement, "100644")})
        changes[0]["before"] = "f" * 40
        with self.assertRaisesRegex(ValueError, "does not match upstream"):
            expected_files(self.tree, changes)

    def test_incomplete_duplicate_noop_and_nonfile_inventories_are_rejected(self):
        self.tree["truncated"] = True
        with self.assertRaisesRegex(ValueError, "incomplete"):
            expected_files(self.tree, [])
        self.tree["truncated"] = False
        self.tree["tree"].append(self.tree["tree"][0].copy())
        with self.assertRaisesRegex(ValueError, "duplicate"):
            expected_files(self.tree, [])
        self.tree["tree"].pop()
        noop = {"path": "src/lib.rs", "before": self.original, "after": self.original, "mode": "100644"}
        with self.assertRaisesRegex(ValueError, "not a source change"):
            expected_files(self.tree, [noop])
        noop["after"] = "e" * 40
        with self.assertRaisesRegex(ValueError, "duplicate local"):
            expected_files(self.tree, [noop, noop])
        self.tree["tree"][0]["mode"] = "120000"
        with self.assertRaisesRegex(ValueError, "regular source"):
            expected_files(self.tree, [])

    def test_ledger_checksum_mismatch_and_symlink_input_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            checked_input(self.root, "src/lib.rs", "0" * 64)
        (self.root / "linked.rs").symlink_to(self.file)
        with self.assertRaisesRegex(ValueError, "symlink"):
            checked_input(self.root, "linked.rs", "0" * 64)


if __name__ == "__main__":
    unittest.main(verbosity=2)
