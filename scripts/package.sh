#!/bin/sh
# Builds the component and the plugin .zip: dist/qr-code-<version>.zip
set -eu
cd "$(dirname "$0")/.."

version=$(sed -n 's/^  "version": "\(.*\)",/\1/p' qr-code/plugin.json)
crate=$(sed -n 's/^version = "\(.*\)"/\1/p' qr-code-src/Cargo.toml | head -n1)
[ -n "$version" ] || { echo "no version in plugin.json" >&2; exit 1; }
[ "$version" = "$crate" ] || { echo "plugin.json ($version) and Cargo.toml ($crate) versions differ" >&2; exit 1; }

cargo build --release --locked --manifest-path qr-code-src/Cargo.toml --target wasm32-unknown-unknown
wasm-tools component new qr-code-src/target/wasm32-unknown-unknown/release/qr_code_src.wasm -o qr-code/qr-code.wasm
wasm-tools validate --features component-model qr-code/qr-code.wasm

mkdir -p dist
zip="dist/qr-code-$version.zip"
rm -f "$zip"
# The package folder is named after the plugin id; the tests are left out.
zip -r -X "$zip" qr-code -x 'qr-code/tests/*' 'qr-code/.*'
echo "$zip"
