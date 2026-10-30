#!/bin/sh
# Fetches the model files Buntini bundles, at pinned revisions, and checks every hash.
# Run once after cloning, and in CI before building or testing; the app and
# ./scripts/check.sh never download models themselves.
#
# Memory search: BAAI/bge-small-en-v1.5 (MIT), converted to float16.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
DIR="$ROOT/models/bge-small-en-v1.5"
REVISION=5c38ec7c405ec4b44b94cc5a9bb96e735b38267a
BASE="https://huggingface.co/BAAI/bge-small-en-v1.5/resolve/$REVISION"
CONFIG=094f8e891b932f2000c92cfc663bac4c62069f5d8af5b5278c4306aef3084750
TOKENIZER=d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66
WEIGHTS_F32=3c9f31665447c8911517620762200d2245a2518d6e7208acc78cd9db317e21ad
WEIGHTS_F16=cc516f81d3a84de5aeca9bf4f14e34bf9ef7a7de80cf719bb08aaa81b6d47fb0

hash() { shasum -a 256 "$1" | cut -d' ' -f1; }
ready() { [ -f "$1" ] && [ "$(hash "$1")" = "$2" ]; }

if ready "$DIR/config.json" "$CONFIG" && ready "$DIR/tokenizer.json" "$TOKENIZER" \
  && ready "$DIR/model.safetensors" "$WEIGHTS_F16"; then
  echo "Memory search model is ready."
  exit 0
fi

mkdir -p "$DIR"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
fetch() {
  curl -sSfL --retry 2 --max-time 600 -o "$WORK/$1" "$BASE/$1"
  if [ "$(hash "$WORK/$1")" != "$2" ]; then
    echo "The downloaded $1 doesn't match its pinned hash. Nothing was installed." >&2
    exit 1
  fi
}
fetch config.json "$CONFIG"
fetch tokenizer.json "$TOKENIZER"
fetch model.safetensors "$WEIGHTS_F32"
python3 -I "$ROOT/scripts/f16_safetensors.py" "$WORK/model.safetensors" "$WORK/model.f16.safetensors"
if [ "$(hash "$WORK/model.f16.safetensors")" != "$WEIGHTS_F16" ]; then
  echo "The converted model doesn't match its pinned hash. Nothing was installed." >&2
  exit 1
fi
mv "$WORK/config.json" "$WORK/tokenizer.json" "$DIR/"
mv "$WORK/model.f16.safetensors" "$DIR/model.safetensors"
cat > "$DIR/NOTICE" <<'NOTICE'
bge-small-en-v1.5 by the Beijing Academy of Artificial Intelligence (BAAI),
https://huggingface.co/BAAI/bge-small-en-v1.5, revision 5c38ec7c405ec4b44b94cc5a9bb96e735b38267a.
Licensed under the MIT License. Weights converted from float32 to float16 for Buntini.
NOTICE
echo "Memory search model is ready."
