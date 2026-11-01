# Apple signing and notarization setup

The app already has a valid local Developer ID Application signing identity, and the locally signed app and both helpers passed signature, team, secure-timestamp, architecture and entitlement checks. Creating another certificate or exporting the private key is unnecessary for local release preparation. Apple notarization still requires an authorized account or a saved `notarytool` Keychain profile. Signing and notarization do not call an AI provider.

## Finish on this Mac

Use the Apple account associated with the same Developer Program team as the signing certificate. It needs an active membership, two-factor authentication and access to notarization. If that account already has a working `notarytool` profile, provide only its profile name. Do not provide a password, certificate or private key in chat.

Otherwise, create an app-specific password at [Apple Account](https://account.apple.com/) under **Sign-In and Security → App-Specific Passwords**, then run this yourself in Terminal:

```sh
xcrun notarytool store-credentials "buntini-notary" --team-id YOURTEAMID
```

Replace `YOURTEAMID` with the ten-character team identifier from the Developer ID identity. The command prompts for the Apple account and app-specific password, validates them with Apple, then saves a local Keychain profile. Use the secure prompt instead of a password argument or environment variable. Your ordinary Apple account password is not the notarization password. When the command succeeds, provide only `buntini-notary` and that it is ready. [Apple's app-specific password instructions](https://support.apple.com/en-us/102654) and [notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow) describe these credentials.

The prepared local tool consumes that saved profile and an **already signed** app:

```sh
sh scripts/release-notarize.sh \
  --app /absolute/path/Buntini.app \
  --profile buntini-notary \
  --identity 'Developer ID Application: Your Name (YOURTEAMID)' \
  --team YOURTEAMID \
  --tag v0.1.0 \
  --output /absolute/path/new-candidate-directory
```

Use the exact identity and matching source version. The output directory must not exist. The tool copies the input app without modifying it, checks the bundle against the checked-out source, notarizes/staples the app, creates a disk image containing the stapled app, signs/notarizes/staples the image, and checks both with Gatekeeper. It produces a zip, DMG, notarization receipts, build metadata and SHA-256 checksums. Credentials stay in Keychain. The tool does not create a release or upload artifacts to GitHub.

Each Apple submission waits up to 30 minutes. A timeout is not a rejection: Apple can continue processing. A failure leaves `.INCOMPLETE` and any submission receipt in the output directory. Inspect the receipt and use `xcrun notarytool wait` or `log` with its submission ID and the same profile before deciding whether to upload again. The verifier accepts only an `Accepted` result with a valid submission ID; an error, pending result or missing receipt cannot become a completed candidate.

To validate a candidate without changing or submitting it:

```sh
APPLE_TEAM_ID=YOURTEAMID RELEASE_TAG=v0.1.0 sh scripts/verify-release.sh \
  /absolute/path/Buntini.app /absolute/path/Buntini.dmg
```

Use the same reviewed source checkout and run outside a restrictive sandbox so macOS trust services can validate the certificate. This checks versions, bundle identifier, purpose strings, exact resource hashes, all three executables, signing team, secure timestamps, approved entitlements, hardened runtime, architecture, notarization tickets and Gatekeeper. Fresh-device and model acceptance still follow [the release runbook](production-readiness.md).

The final verifier also mounts the disk image read-only without opening Finder or executing its contents, verifies the contained app, compares its complete file tree (including signatures and the stapled ticket) with the supplied app, and detaches on success or failure. App roots must contain only `Contents`. A disk-image root may contain only `Buntini.app`, an optional `Applications` symlink exactly to `/Applications`, and bounded non-executable Finder decorations: a valid `.DS_Store` header and a `.VolumeIcon.icns` identical to the reviewed source icon. Custom backgrounds and other entries fail the current policy. A detach failure stops verification and reports the retained mount location without recursively deleting it.

## Enable signing on GitHub Actions

Hosted runners cannot use this Mac's Keychain. CI additionally needs an exportable Developer ID Application certificate **with its private key** in a password-protected `.p12`, plus notarization credentials. A downloaded `.cer` alone is insufficient. In **Keychain Access → My Certificates**, expand the identity, confirm that its private key exists, and export the identity as `.p12` with a strong export password. Keep the export in a private location outside the repository. [Tauri's signing guide](https://v2.tauri.app/distribute/sign/macos/) describes the format and supported credential names.

The protected GitHub `release` environment uses these six fields:

| Secret | Value | Current setup |
| --- | --- | --- |
| `APPLE_SIGNING_IDENTITY` | Exact Developer ID Application identity name | Configured from the verified local identity |
| `APPLE_TEAM_ID` | Ten-character team identifier | Configured |
| `APPLE_CERTIFICATE` | One base64 line containing the exported `.p12`, including its private key | Still needed for hosted CI |
| `APPLE_CERTIFICATE_PASSWORD` | Password chosen when exporting the `.p12` | Still needed for hosted CI |
| `APPLE_ID` | Authorized Apple account email | Still needed for hosted CI |
| `APPLE_PASSWORD` | App-specific password for that account | Still needed for hosted CI |

`scripts/setup-release-secrets.sh` sets all six in one go. Run it yourself in Terminal with an authenticated GitHub CLI. It:

1. Finds the Developer ID identity and team ID in your Keychain.
2. Exports that identity alone as a `.p12` under a new random password. macOS asks you to allow the export.
3. Asks for your Apple ID, then the app-specific password without echoing it.
4. Asks you to confirm, then pipes all six values into the `release` environment.

Nothing is printed, passed as a command argument or kept on disk. `--dry-run` checks everything without setting anything, and `--p12 PATH` uses a `.p12` you exported from Keychain Access instead.

```sh
sh scripts/setup-release-secrets.sh --dry-run
sh scripts/setup-release-secrets.sh
```

To set them by hand instead, use the repository's **Settings → Environments → release → Environment secrets** interface, or run these commands yourself with an authenticated GitHub CLI. These commands change the environment secrets; none should be pasted with real values into chat:

```sh
base64 -i /absolute/private/path/DeveloperID.p12 | tr -d '\r\n' | \
  gh secret set APPLE_CERTIFICATE --repo sagar8080/buntini --env release
gh secret set APPLE_CERTIFICATE_PASSWORD --repo sagar8080/buntini --env release
gh secret set APPLE_ID --repo sagar8080/buntini --env release
gh secret set APPLE_PASSWORD --repo sagar8080/buntini --env release
```

The last three commands prompt for values. The certificate travels directly through a pipe, rather than a terminal printout, shell history argument or committed file. Keep `.p12`, its password and the app-specific password out of logs, support bundles and release assets. Rotate/revoke them if exposed.

The release environment currently requires review by the repository's sole maintainer, allows that maintainer to approve their own run, and disables administrator bypass. This provides an explicit release gate; it is not independent review by another person. Add another trusted maintainer if independent approval is required. Protected branch/tag rules and dependency/secret-scanning settings supplement that gate.

The workflow fails closed on missing/malformed credentials, version/lockfile mismatch, signing errors, rejected or incomplete notarization, unexpected packaged files, resource changes, entitlement drift, failed ticket validation or Gatekeeper rejection. A successful tagged run prepares a **draft** release. Publishing remains the final action after acceptance.
