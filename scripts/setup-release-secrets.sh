#!/bin/sh
# Sets the six Apple secrets the GitHub `release` environment needs to sign and notarize
# Buntini in CI (docs/apple-signing.md). Values travel from this Mac's Keychain and your
# answers into `gh secret set` through pipes: never printed, never passed as command
# arguments, never kept outside a private temporary folder that is removed on exit.
set -eu

usage() {
  cat <<'HELP'
Usage (macOS, with an authenticated GitHub CLI):
  sh scripts/setup-release-secrets.sh [--p12 /path/DeveloperID.p12] [--keychain PATH]
                                      [--repo OWNER/NAME] [--env NAME] [--dry-run]

Finds your Developer ID Application identity and team ID in the Keychain, exports that
one certificate and its private key as a .p12 with a new random password (macOS asks you
to allow the export), asks for your Apple ID and an app-specific password, then sets:

  APPLE_SIGNING_IDENTITY  APPLE_TEAM_ID  APPLE_CERTIFICATE  APPLE_CERTIFICATE_PASSWORD
  APPLE_ID  APPLE_PASSWORD

in the repository's `release` environment. Running it again replaces the values.

  --p12 PATH       Use an identity you exported from Keychain Access instead; asks for
                   the password you chose when exporting it.
  --keychain PATH  Export from this keychain (default: your login keychain).
  --repo OWNER/NAME  Default: the repository of the current checkout.
  --env NAME       Default: release.
  --dry-run        Check everything and say what would be set, but set nothing.

Create the app-specific password at https://account.apple.com under Sign-In and
Security -> App-Specific Passwords. Never paste any of these values into chat.
HELP
}

P12=
KEYCHAIN=
REPO=
ENVIRONMENT=release
DRY_RUN=false
while [ "$#" -gt 0 ]; do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --dry-run) DRY_RUN=true; shift ;;
    --p12|--keychain|--repo|--env)
      if [ "$#" -lt 2 ] || [ -z "$2" ]; then echo "Missing value for $1." >&2; exit 2; fi
      case "$1" in
        --p12) P12=$2 ;;
        --keychain) KEYCHAIN=$2 ;;
        --repo) REPO=$2 ;;
        --env) ENVIRONMENT=$2 ;;
      esac
      shift 2 ;;
    *) echo "Unknown option $1. Use --help." >&2; exit 2 ;;
  esac
done

fail() { echo "$*" >&2; exit 1; }
[ "$(uname -s)" = "Darwin" ] || fail "This needs macOS and its Keychain."
# LibreSSL writes .p12 files that every macOS `security import` accepts.
OPENSSL=/usr/bin/openssl
for tool in security gh base64 awk "$OPENSSL"; do
  command -v "$tool" >/dev/null 2>&1 || fail "Missing $tool."
done
[ -z "$P12" ] || [ -f "$P12" ] || fail "No file at $P12."
if [ "$DRY_RUN" = false ]; then gh auth status >/dev/null 2>&1 || fail "Sign in to GitHub first: gh auth login"; fi
if [ -z "$REPO" ]; then REPO=$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null) || fail "Couldn't tell which repository this is. Pass --repo OWNER/NAME."; fi

# Private scratch space; everything in it is removed however the script ends.
umask 077
WORK=$(mktemp -d "${TMPDIR:-/tmp}/buntini-secrets.XXXXXX")
cleanup() { [ -t 0 ] && stty echo 2>/dev/null; rm -rf "$WORK"; }
trap cleanup EXIT
trap 'exit 130' INT TERM

ask() { # prompt; answer in ANSWER
  printf '%s' "$1" >&2
  IFS= read -r ANSWER || ANSWER=
}
ask_hidden() { # prompt; answer in ANSWER, not echoed
  printf '%s' "$1" >&2
  if [ -t 0 ]; then stty -echo; fi
  IFS= read -r ANSWER || ANSWER=
  if [ -t 0 ]; then stty echo; printf '\n' >&2; fi
}

# 1. The signing identity and its team.
if [ -n "$KEYCHAIN" ]; then
  security find-identity -v -p codesigning "$KEYCHAIN" > "$WORK/identities" 2>/dev/null || true
else
  security find-identity -v -p codesigning > "$WORK/identities" 2>/dev/null || true
fi
grep '"Developer ID Application: ' "$WORK/identities" > "$WORK/developer-ids" || true
COUNT=$(awk 'END { print NR }' "$WORK/developer-ids")
[ "$COUNT" -gt 0 ] || fail "No valid Developer ID Application identity in the Keychain. See docs/apple-signing.md."
CHOICE=1
if [ "$COUNT" -gt 1 ]; then
  echo "More than one Developer ID Application identity:" >&2
  awk -F'"' '{ printf "  %d) %s\n", NR, $2 }' "$WORK/developer-ids" >&2
  ask "Which one? [1-$COUNT] "
  CHOICE=$ANSWER
  case "$CHOICE" in ''|*[!0-9]*) fail "Choose a number." ;; esac
  [ "$CHOICE" -ge 1 ] && [ "$CHOICE" -le "$COUNT" ] || fail "Choose 1 to $COUNT."
fi
LINE=$(awk -v n="$CHOICE" 'NR == n' "$WORK/developer-ids")
SIGNING_IDENTITY=$(printf '%s\n' "$LINE" | awk -F'"' '{ print $2 }')
SHA1=$(printf '%s\n' "$LINE" | awk '{ print toupper($2) }')
TEAM_ID=$(printf '%s\n' "$SIGNING_IDENTITY" | sed -n 's/.*(\([A-Z0-9]\{10\}\))$/\1/p')
[ -n "$TEAM_ID" ] || fail "Couldn't read a team ID from \"$SIGNING_IDENTITY\"."

# 2. The certificate and its private key as a password-protected .p12.
PASSFILE="$WORK/p12-password"
FINAL="$WORK/DeveloperID.p12"
fingerprint() { "$OPENSSL" x509 -in "$1" -noout -fingerprint -sha1 | sed 's/.*=//; s/://g' | tr 'a-f' 'A-F'; }
split_pem() { # bundle prefix: writes prefix-1.pem, prefix-2.pem…
  awk -v prefix="$2" '/-----BEGIN /{ n++; f = prefix "-" n ".pem" } f { print > f } /-----END /{ close(f); f = "" }' "$1"
}
if [ -n "$P12" ]; then
  ask_hidden "Password you chose when exporting $(basename "$P12"): "
  printf '%s' "$ANSWER" > "$PASSFILE"
  cp "$P12" "$FINAL"
else
  echo "Exporting \"$SIGNING_IDENTITY\". macOS asks you to allow it, once for each signing key in the keychain." >&2
  # `security` exports every identity in the keychain at once, under a throwaway password;
  # only the chosen certificate and the key that matches it are kept.
  BUNDLE_PASS=$("$OPENSSL" rand -hex 24)
  printf '%s' "$BUNDLE_PASS" > "$WORK/bundle-password"
  if [ -n "$KEYCHAIN" ]; then
    security export -k "$KEYCHAIN" -t identities -f pkcs12 -P "$BUNDLE_PASS" -o "$WORK/all.p12" >/dev/null 2>&1 || fail "The Keychain export was refused. Export the identity from Keychain Access and pass --p12."
  else
    security export -t identities -f pkcs12 -P "$BUNDLE_PASS" -o "$WORK/all.p12" >/dev/null 2>&1 || fail "The Keychain export was refused. Export the identity from Keychain Access and pass --p12."
  fi
  "$OPENSSL" pkcs12 -in "$WORK/all.p12" -passin file:"$WORK/bundle-password" -nokeys -out "$WORK/certs.pem" 2>/dev/null || fail "Couldn't read the Keychain export. Export the identity from Keychain Access and pass --p12."
  "$OPENSSL" pkcs12 -in "$WORK/all.p12" -passin file:"$WORK/bundle-password" -nocerts -nodes -out "$WORK/keys.pem" 2>/dev/null || fail "Couldn't read the keys in the Keychain export. Export the identity from Keychain Access and pass --p12."
  rm -f "$WORK/all.p12"
  split_pem "$WORK/certs.pem" "$WORK/cert"
  split_pem "$WORK/keys.pem" "$WORK/key"
  rm -f "$WORK/keys.pem"
  CERT=
  for file in "$WORK"/cert-*.pem; do
    [ -f "$file" ] && [ "$(fingerprint "$file")" = "$SHA1" ] && { CERT=$file; break; }
  done
  [ -n "$CERT" ] || fail "The export didn't include \"$SIGNING_IDENTITY\". Export it from Keychain Access and pass --p12."
  "$OPENSSL" x509 -in "$CERT" -noout -pubkey > "$WORK/cert-public.pem"
  KEY=
  for file in "$WORK"/key-*.pem; do
    [ -f "$file" ] || continue
    "$OPENSSL" pkey -in "$file" -pubout > "$WORK/key-public.pem" 2>/dev/null || continue
    cmp -s "$WORK/cert-public.pem" "$WORK/key-public.pem" && { KEY=$file; break; }
  done
  [ -n "$KEY" ] || fail "The export didn't include the private key for \"$SIGNING_IDENTITY\"."
  "$OPENSSL" rand -hex 24 | tr -d '\n' > "$PASSFILE"
  "$OPENSSL" pkcs12 -export -in "$CERT" -inkey "$KEY" -name "$SIGNING_IDENTITY" -passout file:"$PASSFILE" -out "$FINAL"
  rm -f "$WORK"/key-*.pem "$WORK"/cert-*.pem "$WORK/certs.pem" "$WORK/bundle-password"
fi
# Whichever way it came, it must open with its password and hold this one identity.
"$OPENSSL" pkcs12 -in "$FINAL" -passin file:"$PASSFILE" -nokeys -clcerts -out "$WORK/check-cert.pem" 2>/dev/null || fail "The .p12 doesn't open with that password."
[ "$(fingerprint "$WORK/check-cert.pem")" = "$SHA1" ] || fail "The .p12 holds a different certificate from \"$SIGNING_IDENTITY\"."
KEYS=$("$OPENSSL" pkcs12 -in "$FINAL" -passin file:"$PASSFILE" -nocerts -nodes 2>/dev/null | grep -c 'PRIVATE KEY-----$' || true)
[ "$KEYS" -ge 1 ] || fail "The .p12 has no private key. Export the identity (certificate and key), not just the certificate."

# 3. The Apple account that notarizes.
ask "Apple ID (the account in the same developer team): "
APPLE_ID=$ANSWER
case "$APPLE_ID" in *?@?*.?*) ;; *) fail "That doesn't look like an Apple ID email." ;; esac
ask_hidden "App-specific password for $APPLE_ID: "
APPLE_PASSWORD=$ANSWER
[ -n "$APPLE_PASSWORD" ] || fail "No password given."
if ! printf '%s' "$APPLE_PASSWORD" | grep -Eq '^[a-z]{4}-[a-z]{4}-[a-z]{4}-[a-z]{4}$'; then
  ask "That doesn't look like an app-specific password (abcd-efgh-ijkl-mnop). Use it anyway? [y/N] "
  case "$ANSWER" in y|Y|yes) ;; *) fail "Nothing was set." ;; esac
fi

# 4. Confirm, then set.
cat >&2 <<SUMMARY

Repository:  $REPO
Environment: $ENVIRONMENT
Identity:    $SIGNING_IDENTITY
Team ID:     $TEAM_ID
Apple ID:    $APPLE_ID
SUMMARY
if [ "$DRY_RUN" = true ]; then
  echo "Dry run: would set APPLE_SIGNING_IDENTITY, APPLE_TEAM_ID, APPLE_CERTIFICATE ($(base64 -i "$FINAL" | tr -d '\r\n' | awk '{ print length }') base64 characters), APPLE_CERTIFICATE_PASSWORD, APPLE_ID and APPLE_PASSWORD. Nothing was set." >&2
  exit 0
fi
ask "Set these six secrets? [y/N] "
case "$ANSWER" in y|Y|yes) ;; *) fail "Nothing was set." ;; esac

put() { gh secret set "$1" --repo "$REPO" --env "$ENVIRONMENT" >/dev/null || fail "Setting $1 failed; secrets set before it were kept. Run again to finish."; }
printf '%s' "$SIGNING_IDENTITY" | put APPLE_SIGNING_IDENTITY
printf '%s' "$TEAM_ID" | put APPLE_TEAM_ID
base64 -i "$FINAL" | tr -d '\r\n' | put APPLE_CERTIFICATE
put APPLE_CERTIFICATE_PASSWORD < "$PASSFILE"
printf '%s' "$APPLE_ID" | put APPLE_ID
printf '%s' "$APPLE_PASSWORD" | put APPLE_PASSWORD
APPLE_PASSWORD=
echo "Set all six secrets in $REPO ($ENVIRONMENT). The exported .p12 and its password exist only there now; nothing was kept on this Mac." >&2
gh secret list --repo "$REPO" --env "$ENVIRONMENT" >&2 || true
