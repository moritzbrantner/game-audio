#!/usr/bin/env bash
# Builds the wasm facade and writes the wasm-bindgen output to web/pkg/.
# Needs the wasm32-unknown-unknown target and wasm-bindgen-cli matching Cargo.lock.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

want="$(awk '/^name = "wasm-bindgen"$/{getline; gsub(/version = |"/, ""); print; exit}' Cargo.lock)"
have="$(wasm-bindgen --version | awk '{print $2}')"
if [[ "$want" != "$have" ]]; then
  echo "wasm-bindgen-cli $have does not match Cargo.lock $want (cargo install wasm-bindgen-cli --version $want --locked)" >&2
  exit 1
fi

cargo build --locked --release -p game-audio-wasm --target wasm32-unknown-unknown
rm -rf web/pkg
wasm-bindgen --target web --out-dir web/pkg \
  target/wasm32-unknown-unknown/release/game_audio_wasm.wasm
rm -f web/pkg/.gitignore
ls -l web/pkg
