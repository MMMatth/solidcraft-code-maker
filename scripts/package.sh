#!/bin/sh
# Builds the component and the plugin .zip: dist/code-maker-<version>.zip
set -eu
cd "$(dirname "$0")/.."

version=$(sed -n 's/^  "version": "\(.*\)",/\1/p' code-maker/plugin.json)
crate=$(sed -n 's/^version = "\(.*\)"/\1/p' code-maker-src/Cargo.toml | head -n1)
[ -n "$version" ] || { echo "no version in plugin.json" >&2; exit 1; }
[ "$version" = "$crate" ] || { echo "plugin.json ($version) and Cargo.toml ($crate) versions differ" >&2; exit 1; }

rm -f code-maker/*.wasm  # no stale component from an earlier build or name
cargo build --release --locked --manifest-path code-maker-src/Cargo.toml --target wasm32-unknown-unknown
wasm-tools component new code-maker-src/target/wasm32-unknown-unknown/release/code_maker_src.wasm -o code-maker/code-maker.wasm
wasm-tools validate --features component-model code-maker/code-maker.wasm

mkdir -p dist
zip="dist/code-maker-$version.zip"
rm -f "$zip"
# The package folder is named after the plugin id; the tests are left out.
zip -r -X "$zip" code-maker -x 'code-maker/tests/*' 'code-maker/.*'
echo "$zip"
