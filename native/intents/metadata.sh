#!/bin/sh
# Makes what the system reads about Buntini's App Intents (Shortcuts and Spotlight) from
# native/surface/Sources/BuntiniIntents/Intents.swift, without an Xcode project:
# - Metadata.appintents, by Xcode's own metadata processor from the Swift compiler's constants;
# - Localizable/<language>.lproj/Localizable.strings, the intents' names in every language, from
#   the "intents.*" keys of locales/<language>.json.
# Run it after changing an intent; both are committed, so a build needs no Xcode. Needs Xcode.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root="$here/../.."
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
sdk=$(xcrun --show-sdk-path)
toolchain="$DEVELOPER_DIR/Toolchains/XcodeDefault.xctoolchain"
build=$(xcodebuild -version | awk '/Build version/ { print $3 }')
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp "$root/native/surface/Sources/BuntiniIntents/Intents.swift" "$work/Intents.swift"
cd "$work"
printf '%s' '["AppIntent","EntityQuery","AppEntity","TransientEntity","AppEnum","AppShortcutProviding","AppShortcutsProvider","AnyResolverProviding","AppIntentsPackage","DynamicOptionsProvider","_IntentValueRepresentable","_AssistantIntentsProvider","_GenerativeFunctionExtractable","IntentValueQuery","Resolver","AppExtension"]' > protocols.json
xcrun swiftc -parse-as-library -wmo -module-name BuntiniIntents -target arm64-apple-macos13.0 -sdk "$sdk" \
  -c Intents.swift -o Intents.o \
  -emit-const-values-path Intents.swiftconstvalues -Xfrontend -const-gather-protocols-file -Xfrontend protocols.json
echo "Intents.swift" > sources.txt
echo "Intents.swiftconstvalues" > constants.txt
mkdir out
"$toolchain/usr/bin/appintentsmetadataprocessor" --output out --toolchain-dir "$toolchain" --module-name BuntiniIntents \
  --sdk-root "$sdk" --xcode-version "$build" --platform-family macOS --deployment-target 13.0 \
  --target-triple arm64-apple-macos13.0 --source-file-list sources.txt --swift-const-vals-list constants.txt --force
rm -rf "$here/Metadata.appintents"
cp -R out/Metadata.appintents "$here/Metadata.appintents"
python3 -I - "$root/locales" "$here/Localizable" <<'PY'
import json, pathlib, sys
locales, target = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
keys = {"Ask Buntini": "intents.askBuntini", "Question": "intents.question", "Today on the Mac": "intents.today",
        "Start Focus Timer": "intents.startFocus", "Minutes": "intents.minutes", "Keep Mac Awake": "intents.keepAwake"}
for path in sorted(locales.glob("*.json")):
    words = json.loads(path.read_text(encoding="utf-8"))
    folder = target / f"{path.stem}.lproj"
    folder.mkdir(parents=True, exist_ok=True)
    lines = [f'{json.dumps(english, ensure_ascii=False)} = {json.dumps(words[key], ensure_ascii=False)};' for english, key in keys.items()]
    (folder / "Localizable.strings").write_text("\n".join(lines) + "\n", encoding="utf-8")
PY
echo "Made $here/Metadata.appintents and $(ls "$here/Localizable" | wc -l | tr -d ' ') languages."
