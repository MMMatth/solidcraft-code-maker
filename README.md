# QR Code plugin for SolidCraft

A [SolidCraft](https://github.com/tangency-labs/solidcraft) command plugin that adds a plate carrying a
QR code, with the code either raised from the plate or engraved into it. The result is ordinary
editable geometry, ready for 3D printing.

## Use

1. Download `qr-code-<version>.zip` from the [Releases](../../releases) page.
2. In SolidCraft, open **Preferences > Plugins > Install plugin…**, pick the zip and review the
   permissions. The plugin only asks to run `solid.box` and `solid.combine`; it needs no files and no network.
3. Run **QR Code** from the **Plugins** group on the Model tab and fill in the form.

| Field | Default | Meaning |
|---|---|---|
| Text or URL | `https://example.com` | What the code encodes (at most 500 bytes) |
| Error correction | `M` | `L`, `M`, `Q` or `H` |
| Mode | `relief` | `relief` raises the modules; `engrave` cuts them into the plate |
| Module size | 1 mm | Side of one QR module |
| Relief height / engraving depth | 0.6 mm | Must be below the plate thickness when engraving |
| Plate thickness | 2 mm | |
| Margin (modules) | 2 | Blank border around the code |

One run is one undo step, and an invalid input changes nothing.

From a script or an MCP client, the entry is `qr-code.qr` with the same field names as parameters.

## How it works

The QR matrix comes from the [`qrcodegen`](https://crates.io/crates/qrcodegen) crate. Neighbouring dark
modules are merged into as few rectangles as a greedy sweep finds, each rectangle becomes a box, and one
`solid.combine` joins them to the plate (or cuts them out of it). A single combine keeps the run fast:
joining or cutting each box separately gets slower with every box already in the plate.

## Build

You need Rust with the `wasm32-unknown-unknown` target and `wasm-tools`:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-tools
scripts/package.sh        # writes dist/qr-code-<version>.zip
```

Check and test the package with `solidcraft-cli`:

```sh
solidcraft-cli plugin check qr-code
solidcraft-cli plugin test qr-code
```

The layout follows SolidCraft's plugin guide: `qr-code/` is the package (manifest and component),
`qr-code-src/` the Rust crate, and `qr-code-src/wit/plugin.wit` a copy of SolidCraft's `craft:plugin@0.2.0`
interface.

## Releases

CI builds the zip on every push and pull request. Pushing a tag such as `v0.1.0` publishes a release with
the zip and its checksum; the tag must match the `version` in `qr-code/plugin.json` and `qr-code-src/Cargo.toml`.

## Licence

TODO: choose a licence. `qrcodegen` is MIT-licensed.
