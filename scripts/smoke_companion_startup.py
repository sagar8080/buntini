#!/usr/bin/env python3
"""Build and launch a disposable companion to exercise real WebView IPC guards.

The source copy uses unique compile-time credential/migration namespaces, temp
storage, no desktop awareness, and nonpersistent WebViews. Production sources,
credentials, profiles, release bundles and ACL rules are never modified.
"""

import argparse
import hashlib
import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile
import uuid


def replace_once(path, before, after):
    text = path.read_text()
    if text.count(before) != 1:
        raise RuntimeError(f"Expected one instrumentation anchor in {path.name}")
    path.write_text(text.replace(before, after, 1))


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(root, cached_target):
    with tempfile.TemporaryDirectory(prefix="buntini-startup-smoke-") as temporary:
        work = Path(temporary)
        source = work / "source"
        source.mkdir()
        for directory in ("src-tauri", "dist", "worker", "native"):
            shutil.copytree(root / directory, source / directory, ignore=shutil.ignore_patterns("target", "gen", "__pycache__", ".build"))
        identifier = "app.buntini.smoke." + uuid.uuid4().hex
        config_path = source / "src-tauri/tauri.conf.json"
        config = json.loads(config_path.read_text())
        config["identifier"] = identifier
        config["productName"] = "Buntini isolated startup smoke"
        config_path.write_text(json.dumps(config, indent=2))
        code = source / "src-tauri/src"
        replace_once(code / "web.rs", 'const KEYCHAIN_SERVICE: &str = "app.buntini.companion";', f'const KEYCHAIN_SERVICE: &str = "{identifier}";')
        replace_once(code / "web.rs", 'const LEGACY_KEYCHAIN_SERVICE: &str = "app.nexon.companion";', f'const LEGACY_KEYCHAIN_SERVICE: &str = "{identifier}.legacy";')
        replace_once(code / "migration.rs", 'const LEGACY_ID: &str = "app.nexon.companion";', f'const LEGACY_ID: &str = "{identifier}.legacy";')
        for field in ("pet_roam", "pet_awareness", "pet_nudges"):
            replace_once(code / "types.rs", f"            {field}: true,", f"            {field}: false,")
        replace_once(code / "lib.rs", "mod security;", "mod security;\nmod startup_smoke;")
        replace_once(code / "lib.rs", "            let data = app.path().app_data_dir()?;", f"            let data = std::path::PathBuf::from({json.dumps(str(work / 'data'))});")
        replace_once(code / "lib.rs", "            let cache = app.path().app_cache_dir()?;", f"            let cache = std::path::PathBuf::from({json.dumps(str(work / 'cache'))});")
        replace_once(code / "lib.rs", "                .on_navigation(|url| security::ui_navigation_allowed(url, cfg!(dev)))", "                .incognito(true)\n                .on_navigation(|url| security::ui_navigation_allowed(url, cfg!(dev)))")
        replace_once(code / "lib.rs", "                .build(app)?;\n            Ok(())", "                .build(app)?;\n            startup_smoke::start(app.handle().clone());\n            Ok(())")
        # Match installed release-sidecar resolution, even in the cheaper debug build.
        replace_once(code / "controller.rs", "let native_dir = if cfg!(debug_assertions) {", "let native_dir = if false {")
        shutil.copy2(root / "scripts/smoke_startup_observer.rs", code / "startup_smoke.rs")
        shutil.copy2(root / "scripts/smoke_startup_checks.js", code / "startup_smoke_checks.js")
        target = work / "target"
        if cached_target.exists():
            # APFS clones are independent files; root can build its candidate safely.
            subprocess.run(["/bin/cp", "-cR", str(cached_target), str(target)], check=True)
        build_log = work / "build.log"
        with build_log.open("w") as log:
            built = subprocess.run([
                "cargo", "build", "--locked", "--offline", "--manifest-path", str(source / "src-tauri/Cargo.toml"),
                "--target-dir", str(target), "--features", "custom-protocol",
            ], cwd=source, stdout=log, stderr=subprocess.STDOUT, timeout=900)
        if built.returncode:
            raise RuntimeError("Disposable build failed:\n" + build_log.read_text()[-12000:])
        app = work / "BuntiniSmoke.app"
        macos = app / "Contents/MacOS"
        resources = app / "Contents/Resources"
        macos.mkdir(parents=True)
        resources.mkdir()
        shutil.copy2(target / "debug/buntini", macos / "buntini")
        for helper in ("buntini-fm", "buntini-audio"):
            shutil.copy2(source / "native" / helper, macos / helper)
        shutil.copytree(source / "worker", resources / "worker")
        with (app / "Contents/Info.plist").open("wb") as output:
            plistlib.dump({"CFBundleIdentifier": identifier, "CFBundleExecutable": "buntini", "CFBundleName": "BuntiniSmoke", "CFBundlePackageType": "APPL", "LSUIElement": True, "LSMinimumSystemVersion": "13.0"}, output)
        subprocess.run(["/usr/bin/codesign", "--force", "--deep", "--sign", "-", str(app)], check=True, capture_output=True, text=True)
        binary_hash = digest(macos / "buntini")
        started = subprocess.run([str(macos / "buntini")], cwd=source, capture_output=True, text=True, timeout=90)
        reports = [line.removeprefix("BUNTINI_STARTUP_SMOKE ") for line in started.stdout.splitlines() if line.startswith("BUNTINI_STARTUP_SMOKE ")]
        if len(reports) != 1:
            raise RuntimeError(f"Startup report missing; exit {started.returncode}; stderr: {started.stderr[-6000:]}")
        report = json.loads(reports[0])
        report.update({
            "process_exit_code": started.returncode,
            "disposable_identifier": identifier,
            "binary_sha256": binary_hash,
            "production_security_source_sha256": digest(root / "src-tauri/src/security.rs"),
            "production_capability_sha256": digest(root / "src-tauri/capabilities/default.json"),
            "production_permission_manifest_sha256": digest(root / "src-tauri/permissions/companion.json"),
            "isolation": ["Separate source tree, Cargo target and app bundle", "Unique compile-time Keychain/migration namespaces", "Temporary data/cache", "Desktop awareness/roaming disabled", "Nonpersistent WebViews", "No microphone, TCC prompts, personal apps or provider calls requested"],
            "unchanged_boundaries": ["Production IPC command handlers", "Production explicit app-command permission allowlist", "Production main capability", "Production main navigation/popup/download guards", "Bundled frontend production assets"],
            "limitations": ["Separate debug binary with custom-protocol; not the signed release candidate", "Observer and storage isolation are injected only into the disposable copy", "Does not establish fresh-Mac permissions, notarization or Gatekeeper acceptance"],
        })
        if started.returncode:
            report["passed"] = False
        return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    try:
        result = run(root, root / "src-tauri/target")
    except Exception as error:
        result = {"kind": "disposable_companion_startup_smoke", "passed": False, "error": str(error)}
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    raise SystemExit(0 if result.get("passed") else 1)
