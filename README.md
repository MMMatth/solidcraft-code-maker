# QR Code plugin for SolidCraft

A [SolidCraft](https://github.com/tangency-labs/solidcraft) command plugin that adds a plate carrying a
QR code, with the code either raised from the plate or engraved into it. The result is ordinary
editable geometry, ready for 3D printing.

## Use

1. Download `qr-code-<version>.zip` from the [Releases](../../releases) page.
2. In SolidCraft, open **Preferences > Plugins > Install plugin…**, pick the zip and review the
   permissions. The plugin only asks to run `solid.box`, `solid.combine`, `solid.move` and `model.faces`; it needs no files and no network.
3. Run **QR Code** from the **Plugins** group on the Model tab and fill in the form.

| Field | Default | Meaning |
|---|---|---|
| Text or link | `https://example.com` | What the code says (at most 500 characters) |
| Put it on a face (optional) | none | Click a flat face of your model; leave empty to get a new plate |
| Style | Raised | Raised, or Engraved into the plate or model |
| Size | 30 mm | Width of the code |
| Height or depth | 0.6 mm | How far the code stands out, or how deep it is cut |
| Move away from the face | 0 mm | Positive lifts the code off the face, negative sinks it into the model (needs a face) |

Everything else is chosen for you: a plate 2 mm thick with a small blank border, a level of error
correction that survives small print flaws, and a single body for the whole result, and the code kept as a sketch plus an extrusion so you can still edit it.

With a face picked, the code is centred on the face's centre, upright toward world Z (toward world Y on a
horizontal face), and it joins the picked body (Raised) or is cut into it (Engraved). A code larger than the
face overhangs it, so pick a bigger face or reduce the size. If the size is too small for the text to print
well (a module under 0.4 mm), the plugin says how big it needs to be.

One run is one undo step, and an invalid input changes nothing.

From a script or an MCP client, the entry is `qr-code.qr`, with the parameters `text`, `face`, `style` (`raised` or `engraved`), `size`, `height` and `offset`.

## How it works

The QR matrix comes from the [`qrcodegen`](https://crates.io/crates/qrcodegen) crate. The plugin traces the
outline of every dark region, module by module, so the sketch has the exact pixel shape of the code with no
line between two neighbouring dark modules. It draws those outlines in one sketch named "QR code", then
extrudes it with one feature, also named "QR code". The result is two ordinary entries in the timeline, so you
can edit the sketch, change the extrusion's height or move the sketch afterwards. On a face the sketch lies on
a plane centred on the face (shifted by the offset along its normal); without a face it lies on top of a new
plate.

A plugin may make only 256 host calls in one run and each outline is one call, so a very long text is refused
with a message asking for a shorter one.

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
