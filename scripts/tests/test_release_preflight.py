import importlib.util
import base64
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("release_preflight", Path(__file__).resolve().parents[1] / "release_preflight.py")
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


class ReleasePreflightTests(unittest.TestCase):
    def manifest_root(self, version="1.2.3"):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        (root / "src-tauri").mkdir()
        for path in ("package.json", "src-tauri/tauri.conf.json"):
            (root / path).write_text(json.dumps({"version": version}))
        (root / "package-lock.json").write_text(json.dumps({"version": version, "packages": {"": {"version": version}}}))
        (root / "src-tauri/Cargo.toml").write_text(f'[package]\nname = "buntini"\nversion = "{version}"\n')
        (root / "src-tauri/Cargo.lock").write_text(f'version = 4\n[[package]]\nname = "buntini"\nversion = "{version}"\n')
        return root

    def test_matching_manifests_and_prereleases(self):
        release.validate_versions(self.manifest_root(), "v1.2.3")
        release.validate_versions(self.manifest_root("1.2.3-beta.1"), "v1.2.3-beta.1")

    def test_mismatched_manifest_cannot_ship(self):
        root = self.manifest_root()
        (root / "src-tauri/Cargo.toml").write_text('[package]\nname = "buntini"\nversion = "1.2.2"\n')
        with self.assertRaisesRegex(ValueError, "Cargo.toml"):
            release.validate_versions(root, "v1.2.3")
        with self.assertRaises(ValueError):
            release.validate_versions(root, "v1.2.4")

    def test_mismatched_or_ambiguous_cargo_lock_cannot_ship(self):
        root = self.manifest_root()
        path = root / "src-tauri/Cargo.lock"
        path.write_text('version = 4\n[[package]]\nname = "buntini"\nversion = "1.2.2"\n')
        with self.assertRaisesRegex(ValueError, "Cargo.lock app package"):
            release.validate_versions(root, "v1.2.3")
        path.write_text('version = 4\n[[package]]\nname = "unrelated"\nversion = "1.2.3"\n')
        with self.assertRaisesRegex(ValueError, "exactly one"):
            release.validate_versions(root, "v1.2.3")
        path.write_text('version = 4\n' + '[[package]]\nname = "buntini"\nversion = "1.2.3"\n' * 2)
        with self.assertRaisesRegex(ValueError, "exactly one"):
            release.validate_versions(root, "v1.2.3")

    def test_invalid_tags_rejected(self):
        for tag in ("main", "v1", "v1.2.3/other", "v1.2.3;cmd", "v1.2.3\n"):
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                release.validate_versions(self.manifest_root(), tag)

    def test_missing_credentials_block_release_without_logging_values(self):
        with self.assertRaisesRegex(ValueError, "APPLE_CERTIFICATE_PASSWORD") as error:
            release.validate_signing({"APPLE_CERTIFICATE": "secret-certificate"})
        self.assertNotIn("secret-certificate", str(error.exception))

    def test_wrong_or_adhoc_identity_rejected(self):
        env = {name: "test-secret" for name in ("APPLE_CERTIFICATE", "APPLE_CERTIFICATE_PASSWORD", "APPLE_ID", "APPLE_PASSWORD")}
        env["APPLE_CERTIFICATE"] = base64.b64encode(b"test-fixture").decode()
        env["APPLE_TEAM_ID"] = "ABCDEFGHIJ"
        for identity in ("-", "Apple Development: Person (ABCDEFGHIJ)", "Developer ID Application: Person (OTHERTEAM0)"):
            with self.subTest(identity=identity), self.assertRaises(ValueError):
                release.validate_signing({**env, "APPLE_SIGNING_IDENTITY": identity})
        release.validate_signing({**env, "APPLE_SIGNING_IDENTITY": "Developer ID Application: Person (ABCDEFGHIJ)"})

    def test_malformed_certificate_and_multiline_inputs_rejected_privately(self):
        env = {name: "test-secret" for name in ("APPLE_CERTIFICATE_PASSWORD", "APPLE_ID", "APPLE_PASSWORD")}
        env.update(APPLE_TEAM_ID="ABCDEFGHIJ", APPLE_SIGNING_IDENTITY="Developer ID Application: Person (ABCDEFGHIJ)")
        for certificate in ("not-base64", "c2VjcmV0\n", "é"):
            with self.subTest(certificate=certificate), self.assertRaises(ValueError) as error:
                release.validate_signing({**env, "APPLE_CERTIFICATE": certificate})
            self.assertNotIn("test-secret", str(error.exception))


if __name__ == "__main__":
    unittest.main()
