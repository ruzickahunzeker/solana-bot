#!/usr/bin/env python3
"""Check the CI-only Cargo-vendored snapshot without using any Git credentials."""
import hashlib
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
REVISION = "064c58b317b9a85f212c0de72caea286fc72fdb4"
CHECKSUMS = {
    "jupiter-0.1.0": "9d102ff9a06080faabd5a822684c80eeac7b12eaea8d0544be38f685fe1de11d",
    "protocol-zap-0.1.0": "0da5f50de4b36477226498b9882c679b10aa5e8523f836c3d134946b45339ab9",
    "zap-sdk-0.1.0": "b5718805bc2a4364b7aec920460323c4b5a2c1c36a7437fabde239fb1f1758cf",
}


class LockedGitSourcesTests(unittest.TestCase):
    def test_only_the_three_locked_packages_are_present(self):
        directory = ROOT / "ci/locked-git-sources"
        self.assertEqual({p.name for p in directory.iterdir()}, set(CHECKSUMS))

    def test_inventory_and_every_file_match_cargo_checksums(self):
        for package, expected in CHECKSUMS.items():
            with self.subTest(package=package):
                directory = ROOT / "ci/locked-git-sources" / package
                checksum = directory / ".cargo-checksum.json"
                self.assertEqual(hashlib.sha256(checksum.read_bytes()).hexdigest(), expected)
                inventory = json.loads(checksum.read_text())
                self.assertIsNone(inventory["package"])
                files = {p.relative_to(directory).as_posix()
                         for p in directory.rglob("*") if p.is_file() and p != checksum}
                self.assertEqual(files, set(inventory["files"]))
                for name, digest in inventory["files"].items():
                    path = directory / name
                    self.assertFalse(path.is_symlink())
                    self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), digest, name)

    def test_ci_replacement_keeps_the_original_git_revision(self):
        config = (ROOT / "ci/locked-git-sources.toml").read_text()
        self.assertIn(f'rev = "{REVISION}"', config)
        self.assertIn('directory = "ci/locked-git-sources"', config)
        self.assertEqual(config.count("replace-with"), 1)
        lock = (ROOT / "Cargo.lock").read_text()
        self.assertEqual(lock.count(f"zap-program?rev={REVISION}#{REVISION}"), 3)
        self.assertFalse((ROOT / ".cargo/config.toml").exists())

    def test_anchor_external_idl_matches_the_same_locked_checkout(self):
        directory = ROOT / "ci/idls"
        self.assertEqual({p.name for p in directory.iterdir()}, {"jupiter.json"})
        idl = directory / "jupiter.json"
        self.assertFalse(idl.is_symlink())
        self.assertEqual(hashlib.sha256(idl.read_bytes()).hexdigest(),
                         "d1dc2e8726886dbdcde16b52bc5dd239779ec4a56f164b9444ac347677267a3e")
        package = ROOT / "ci/locked-git-sources/jupiter-0.1.0"
        resolved = next(p / "idls" for p in package.parents if (p / "idls").exists())
        self.assertEqual(resolved, directory)


if __name__ == "__main__":
    unittest.main(verbosity=2)
