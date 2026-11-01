from pathlib import Path
import shutil
import subprocess
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import release_dmg as image
import test_release_artifact as artifact_fixtures


class ReleaseDmgTests(unittest.TestCase):
    def fixture(self):
        fixture = artifact_fixtures.ReleaseArtifactTests()
        self.addCleanup(fixture.doCleanups)
        root, app, _, _ = fixture.fixture()
        dmg = root / "fixture.dmg"
        dmg.write_bytes(b"synthetic-image")
        return root, app, dmg

    def run_fixture(self, mutate=None, detach_fails=False, attach_fails=False, verifier_fails=False, interrupt=False):
        root, app, dmg = self.fixture()
        calls, verified, mounts = [], [], []
        def runner(args):
            calls.append(args)
            if args[1] == "attach":
                mount = Path(args[args.index("-mountpoint") + 1])
                mounts.append(mount)
                self.addCleanup(shutil.rmtree, mount.parent, ignore_errors=True)
                shutil.copytree(app, mount / "Buntini.app")
                (mount / "Applications").symlink_to("/Applications")
                if mutate:
                    mutate(mount)
                if attach_fails:
                    raise ValueError("partial attach failed")
                if interrupt:
                    raise KeyboardInterrupt()
            elif args[1] == "detach":
                if detach_fails:
                    raise ValueError("detach failed")
                mount = Path(args[2])
                shutil.rmtree(mount)
                mount.mkdir()
            else:
                self.fail("Unexpected tool invocation")
            return subprocess.CompletedProcess(args, 0, b"", b"")
        def verifier(*args):
            verified.append(args)
            if verifier_fails:
                raise ValueError("signature rejected")
        return root, app, dmg, runner, verifier, calls, verified, mounts

    def test_readonly_payload_match_validates_embedded_code_and_detaches(self):
        root, app, dmg, runner, verifier, calls, verified, mounts = self.run_fixture()
        image.verify_image(root, app, dmg, "ABCDEFGHIJ", runner, verifier)
        self.assertEqual(len(verified), 1)
        self.assertEqual(verified[0][0], mounts[0] / "Buntini.app")
        self.assertIn("-readonly", calls[0])
        self.assertIn("-nobrowse", calls[0])
        self.assertIn("-noautoopen", calls[0])
        self.assertEqual(calls[-1][1], "detach")
        self.assertFalse(mounts[0].parent.exists())
        self.assertTrue(all(args[0] == "/usr/bin/hdiutil" for args in calls))

    def test_different_signed_app_or_ticket_is_rejected_and_detached(self):
        for relative in ("Contents/MacOS/buntini", "Contents/_CodeSignature/CodeResources"):
            values = self.run_fixture(mutate=lambda mount: (mount / "Buntini.app" / relative).write_bytes(b"different-artifact"))
            root, app, dmg, runner, verifier, calls, _, _ = values
            with self.subTest(relative=relative), self.assertRaisesRegex(ValueError, "differs from the supplied"):
                image.verify_image(root, app, dmg, "ABCDEFGHIJ", runner, verifier)
            self.assertEqual(calls[-1][1], "detach")

    def test_unexpected_root_payload_and_wrong_applications_link_rejected(self):
        def wrong_link(mount):
            (mount / "Applications").unlink()
            (mount / "Applications").symlink_to("/System/Applications")
        for mutation in (lambda mount: (mount / "install.sh").write_text("touch NEVER_EXECUTED"), wrong_link):
            root, app, dmg, runner, verifier, calls, verified, _ = self.run_fixture(mutate=mutation)
            with self.assertRaises(ValueError):
                image.verify_image(root, app, dmg, "ABCDEFGHIJ", runner, verifier)
            self.assertEqual(calls[-1][1], "detach")
            self.assertEqual(verified, [])
            self.assertTrue(all(args[0] == "/usr/bin/hdiutil" for args in calls))

    def test_partial_attach_signature_failure_and_interrupt_still_detach(self):
        for options, expected in (({"attach_fails": True}, ValueError), ({"verifier_fails": True}, ValueError), ({"interrupt": True}, KeyboardInterrupt)):
            root, app, dmg, runner, verifier, calls, _, mounts = self.run_fixture(**options)
            with self.subTest(options=options), self.assertRaises(expected):
                image.verify_image(root, app, dmg, "ABCDEFGHIJ", runner, verifier)
            self.assertEqual(calls[-1][1], "detach")
            self.assertFalse(mounts[0].parent.exists())

    def test_detach_failure_is_reported_without_recursive_volume_deletion(self):
        root, app, dmg, runner, verifier, _, _, mounts = self.run_fixture(detach_fails=True)
        with self.assertRaisesRegex(ValueError, "detach could not be confirmed"):
            image.verify_image(root, app, dmg, "ABCDEFGHIJ", runner, verifier)
        self.assertTrue((mounts[0] / "Buntini.app/Contents/MacOS/buntini").exists())

    def test_bounded_known_finder_decorations(self):
        def decorated(mount):
            (mount / ".DS_Store").write_bytes(b"\0\0\0\1Bud1" + b"fixture")
            (mount / ".DS_Store").chmod(0o644)
            (mount / ".VolumeIcon.icns").write_bytes(b"icon-fixture")
            (mount / ".VolumeIcon.icns").chmod(0o644)
        root, app, dmg, runner, verifier, _, _, _ = self.run_fixture(mutate=decorated)
        image.verify_image(root, app, dmg, "ABCDEFGHIJ", runner, verifier)
        for mutation in (lambda mount: (mount / ".DS_Store").write_bytes(b"not-Finder-metadata"), lambda mount: (mount / ".VolumeIcon.icns").write_bytes(b"changed-icon")):
            root, app, dmg, runner, verifier, calls, _, _ = self.run_fixture(mutate=mutation)
            with self.assertRaises(ValueError):
                image.verify_image(root, app, dmg, "ABCDEFGHIJ", runner, verifier)
            self.assertEqual(calls[-1][1], "detach")


if __name__ == "__main__":
    unittest.main()
