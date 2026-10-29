import json
from pathlib import Path
import plistlib
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import release_artifact as artifact
from release_notary_result import validate_result


class ReleaseArtifactTests(unittest.TestCase):
    def fixture(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        base = Path(temp.name)
        root, app = base / "repo", base / "Buntini.app"
        (root / "src-tauri/icons").mkdir(parents=True)
        (root / "worker").mkdir()
        config = {
            "version": "1.2.3", "identifier": "app.buntini.companion",
            "bundle": {"resources": {"../worker/main.py": "worker/main.py"}, "icon": ["icons/icon.icns"], "macOS": {"minimumSystemVersion": "13.0"}},
        }
        (root / "src-tauri/tauri.conf.json").write_text(json.dumps(config))
        (root / "package.json").write_text(json.dumps({"version": "1.2.3"}))
        (root / "package-lock.json").write_text(json.dumps({"version": "1.2.3", "packages": {"": {"version": "1.2.3"}}}))
        (root / "src-tauri/Cargo.toml").write_text('[package]\nname = "buntini"\nversion = "1.2.3"\n')
        (root / "src-tauri/Cargo.lock").write_text('version = 4\n[[package]]\nname = "buntini"\nversion = "1.2.3"\n')
        privacy = {"NSMicrophoneUsageDescription": "Only during push to talk", "NSAppleEventsUsageDescription": "Only allowed apps", "LSUIElement": True}
        policy = {"com.apple.security.device.audio-input": True, "com.apple.security.automation.apple-events": True}
        (root / "src-tauri/Info.plist").write_bytes(plistlib.dumps(privacy))
        (root / "src-tauri/Entitlements.plist").write_bytes(plistlib.dumps(policy))
        (root / "worker/main.py").write_text("# reviewed worker\n")
        (root / "src-tauri/icons/icon.icns").write_bytes(b"icon-fixture")
        contents = app / "Contents"
        for directory in ("MacOS", "Resources/worker", "_CodeSignature"):
            (contents / directory).mkdir(parents=True)
        for executable in ("buntini", "buntini-audio", "buntini-fm"):
            binary = contents / "MacOS" / executable
            binary.write_bytes(b"synthetic-binary-fixture")
            binary.chmod(0o755)
        (contents / "_CodeSignature/CodeResources").write_text("signature-fixture")
        (contents / "Resources/worker/main.py").write_bytes((root / "worker/main.py").read_bytes())
        (contents / "Resources/icon.icns").write_bytes(b"icon-fixture")
        info = {**privacy, "CFBundleIdentifier": config["identifier"], "CFBundleShortVersionString": "1.2.3", "CFBundleVersion": "1.2.3", "CFBundlePackageType": "APPL", "LSMinimumSystemVersion": "13.0", "CFBundleExecutable": "buntini", "CFBundleIconFile": "icon.icns"}
        (contents / "Info.plist").write_bytes(plistlib.dumps(info))
        return root, app, info, policy

    def test_reviewed_bundle_layout_is_accepted(self):
        root, app, _, policy = self.fixture()
        self.assertEqual(artifact.validate_bundle(root, app), ("buntini", policy))

    def test_stapled_ticket_layout_is_allowed_pending_native_validation(self):
        root, app, _, policy = self.fixture()
        (app / "Contents/CodeResources").write_bytes(b"synthetic-ticket-fixture")
        self.assertEqual(artifact.validate_bundle(root, app), ("buntini", policy))

    def test_wrong_version_identity_or_permission_description_rejected(self):
        for key, value in (("CFBundleShortVersionString", "0.9.0"), ("CFBundleIdentifier", "app.other"), ("NSMicrophoneUsageDescription", "Always listening")):
            root, app, info, _ = self.fixture()
            info[key] = value
            (app / "Contents/Info.plist").write_bytes(plistlib.dumps(info))
            with self.subTest(key=key), self.assertRaises(ValueError):
                artifact.validate_bundle(root, app)

    def test_modified_worker_or_extra_secret_file_rejected(self):
        root, app, _, _ = self.fixture()
        (app / "Contents/Resources/worker/main.py").write_text("# modified payload")
        with self.assertRaisesRegex(ValueError, "resource differs"):
            artifact.validate_bundle(root, app)
        root, app, _, _ = self.fixture()
        (app / "Contents/.env").write_text("private-fixture")
        with self.assertRaisesRegex(ValueError, "unexpected files") as error:
            artifact.validate_bundle(root, app)
        self.assertNotIn("private-fixture", str(error.exception))

    def test_extra_app_root_file_or_symlinked_contents_rejected(self):
        root, app, _, _ = self.fixture()
        (app / ".env").write_text("private-fixture")
        with self.assertRaisesRegex(ValueError, "App root"):
            artifact.validate_bundle(root, app)
        root, app, _, _ = self.fixture()
        contents = app / "Contents"
        moved = app.parent / "moved-contents"
        contents.rename(moved)
        contents.symlink_to(moved, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "App root"):
            artifact.validate_bundle(root, app)

    def test_missing_helper_or_unexpected_directory_rejected(self):
        root, app, _, _ = self.fixture()
        (app / "Contents/MacOS/buntini-fm").unlink()
        with self.assertRaises(ValueError):
            artifact.validate_bundle(root, app)
        root, app, _, _ = self.fixture()
        (app / "Contents/Resources/.build").mkdir()
        with self.assertRaisesRegex(ValueError, "unexpected directories"):
            artifact.validate_bundle(root, app)

    def test_symlinked_worker_is_rejected(self):
        root, app, _, _ = self.fixture()
        packaged = app / "Contents/Resources/worker/main.py"
        packaged.unlink()
        packaged.symlink_to(root / "worker/main.py")
        with self.assertRaisesRegex(ValueError, "symlinks"):
            artifact.validate_bundle(root, app)

    def test_extra_or_missing_entitlements_rejected(self):
        expected = {"com.apple.security.device.audio-input": True}
        artifact.validate_entitlements(plistlib.dumps(expected), expected)
        for actual in ({}, {**expected, "com.apple.security.get-task-allow": True}, {**expected, "com.apple.security.cs.allow-jit": True}):
            with self.subTest(actual=actual), self.assertRaises(ValueError):
                artifact.validate_entitlements(plistlib.dumps(actual), expected)
        with self.assertRaises(ValueError):
            artifact.validate_entitlements(b"not-a-plist", expected)

    def test_developer_id_signature_requires_team_timestamp_and_runtime(self):
        details = "\n".join(("Identifier=app.buntini.companion", "CodeDirectory v=20500 flags=0x10000(runtime)", "Authority=Developer ID Application: Fixture (ABCDEFGHIJ)", "Timestamp=Oct 29, 2026 at 12:00:00", "TeamIdentifier=ABCDEFGHIJ"))
        artifact.validate_signature(details, "ABCDEFGHIJ", True, "app.buntini.companion")
        invalid = (
            details.replace("TeamIdentifier=ABCDEFGHIJ", "TeamIdentifier=OTHERTEAM0"),
            details.replace("Authority=Developer ID Application:", "Authority=Apple Development:"),
            details.replace("flags=0x10000(runtime)", "flags=0x0(none)"),
            details.replace("Timestamp=Oct 29, 2026 at 12:00:00", "Timestamp="),
            details.replace("Identifier=app.buntini.companion", "Identifier=app.other"),
            details.replace("flags=0x10000(runtime)", "flags=0x10002(adhoc,runtime)"),
            details + "\nSignature=adhoc",
        )
        for value in invalid:
            with self.subTest(value=value), self.assertRaises(ValueError):
                artifact.validate_signature(value, "ABCDEFGHIJ", True, "app.buntini.companion")

    def test_notary_rejection_pending_and_unidentifiable_success_fail_closed(self):
        submission = "3b241101-e2bb-4255-8caf-4136c566a962"
        self.assertEqual(validate_result({"status": "Accepted", "id": submission}), submission)
        for payload in ({"status": "Invalid", "id": submission}, {"status": "In Progress", "id": submission}, {"status": "Accepted"}, {"status": "Accepted", "id": "invalid"}, []):
            with self.subTest(payload=payload), self.assertRaises(ValueError):
                validate_result(payload)

    def test_notarization_cli_help_and_invalid_options_do_not_start_work(self):
        script = Path(__file__).resolve().parents[1] / "release-notarize.sh"
        help_result = subprocess.run(["sh", str(script), "--help"], capture_output=True, text=True)
        self.assertEqual(help_result.returncode, 0)
        self.assertIn("Keychain", help_result.stdout)
        self.assertNotEqual(subprocess.run(["sh", str(script), "--unknown"], capture_output=True).returncode, 0)


if __name__ == "__main__":
    unittest.main()
