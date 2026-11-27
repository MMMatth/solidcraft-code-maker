# Changelog

All notable changes to the plugin. The format follows [Keep a Changelog](https://keepachangelog.com/),
and the versions follow [Semantic Versioning](https://semver.org/).

Version 0.5.0 is the first published release. Versions 0.1.0 to 0.4.0 were development steps of the first
day, kept below for the history: they have no tag and no release.

## [0.7.0] - 2026-11-27

### Added
- An animated picture for the workshop (`docs/demo.webp`), which takes no GIF.
- **Inlay style**: the code is engraved and the engraving is filled flush by a body of its own, so a slicer can print the
  plate and the code in two filaments. The plate keeps its pocket and the code is one body named after it, coloured as
  asked.
- **Code as its own body**: a raised code can also be a body of its own instead of part of the plate or face.
  An engraving that is its own body is an inlay. An inlay lies in its face, so it cannot be lifted or sunk (`move_z`) on a
  face: Move X and Y still work.

## [0.6.1] - 2026-11-27

### Fixed
- **Fixed: the run failed with `no face at [...]` after the code was already drawn.** The host refuses a whole
  colour call when one point of it falls off a face, which happens on thin faces once the point is rounded. The
  plugin now cuts a refused batch in halves until the faces it cannot find are alone, colours the rest, and says in
  the reply how many it left uncoloured instead of failing.

### Added
- A workshop manifest (`solidcraft-workshop.json`) and a screenshot (`docs/screenshot.png`) for SolidCraft's workshop.

## [0.6.0] - 2026-11-26

### Added
- **ArUco markers and AprilTags**: a **Type of code** list (QR code, ArUco marker, AprilTag), a **Family** list and a
  **Marker number**. The form offers the common families: ArUco 4x4, 5x5, 6x6 and original, and AprilTag 36h11, 16h5
  and Standard 41h12. The plugin holds every official ArUco and AprilTag dictionary, taken from OpenCV and the
  AprilTag library, and every one was checked by decoding the drawn markers with OpenCV's ArUco
  detector and the AprilTag library (`scripts/verify-tags.py`).
- **Barcodes**, written by the `rxing` library: Data Matrix, Aztec, PDF417, Code 128, Code 39, Code 93, Codabar,
  EAN-13, EAN-8, UPC-A, UPC-E and ITF, in the same **Type of code** list. A new **Bar height** field (barcodes only)
  sets how tall the bars are, and **Size** is the width of the whole barcode. Each was read back with the ZXing
  reader (`scripts/verify-barcodes.py`). The plugin grows from 0.2 MB to 1.6 MB.
- Markers and barcodes go on a new plate or on a face, raised or engraved, coloured and moved like a QR code, and are named
  `ArUco marker 2`, `EAN-13 2`, and so on when the design has some already.
- The reply says the `kind` of the code.
- A new icon in the isometric style of SolidCraft's own: a plate with a flat raised marker, drawn with the paths, colours and round line ends SolidCraft's icon check requires.

### Changed
- **The plugin is renamed Code Maker** (id `code-maker`, button **Code**), since it is no longer only QR codes. The
  entry is now `code-maker.code`, which SolidCraft's MCP server exposes as `code-maker_code` (it was `qr-code_qr`).
  Remove the old QR Code plugin before installing this one.
- The form starts with **Type of code**, then shows only what that type needs: the text for a QR code, the
  family and the number for an ArUco marker or an AprilTag.
- The fields are in a logical order (type, content, face, look, size, position).
- The "too small" message no longer talks about a shorter text for a marker, which has none.
- **Error correction for a QR code**: Low (7%), Medium (15%, the default), Quartile (25%) or High (30%). A higher level
  reads through more damage or dirt, in a denser code.
- **Fixed: a plate came out dark, with its modules light.** SolidCraft keeps colours by body name, so after a plate was
  undone or deleted, a new plate that took the same name (the first free one) inherited its old colours, and on a
  big plate they landed on its top face. A new plate now starts with none: every face the run made is reset before
  the code is coloured as asked (the reply says how many, in `reset_faces`). A face of a part of yours is never
  touched. The plugin does not read the design's colour table to do it, which outgrows the 1 MiB a reply may hold
  after a few runs and made the run fail with `command result size`.
- **A size that suits each type**: the form starts at 30 mm for a QR code but 65 mm for an EAN-13 or 80 mm for a
  Code 128, which are wide. A size too small to print (modules under 0.4 mm) is no longer refused, which left the
  preview empty and said nothing: it is raised to the least that prints well, and the result says so (`warning`,
  with `width_mm` and `module_mm`). Code 128 has its own field, `Code 128 text`.
- **Each strict barcode has its own field** (EAN-13 number, UPC-A number, Code 39 text, ...) with its rule and a working
  example, and a mistake says what is wrong: a wrong length, a character the code does not allow, or the check digit it
  should have. Spaces and hyphens in a number are ignored.
- **One library for every code**: QR codes now come from `rxing` too, with the medium error correction by default, instead
  of `qrcodegen`. They scan the same, but a few are drawn with another mask, so they differ from 0.5.1's in some modules.
- Internally the plugin draws any grid of dark modules, square or one row of bars, so another
  kind of code needs only a new grid.

## [0.5.1] - 2026-11-25

### Fixed
- A face given by a point on it, which is how an MCP client or a script usually picks one, was refused with
  "the pick has no face index". A face can now be picked by `index`, by `name` or by a `point` on it. With a
  point, the flat face whose plane holds the point is used, and a point on a curved face is refused with a clear
  message.

## [0.5.0] - 2026-11-25

### Added
- An icon in the style of SolidCraft's own, for the button and the dialog.
- A **Colour of the code** list (black by default, blue, red, green, orange, yellow, white, or no colour).
  Only the faces the code adds are coloured: the top and sides of the raised modules, or the floor and walls of the engraving.
- **Move X / Y / Z**, for a new plate or for a code on a face, and **Move along** to use the world axes or the
  face's own (X right, Y up, Z out of the face), so a code can be lifted or sunk on a slanted or vertical face.
- **Rotate**, to turn the code on its face.
- A second code in the same design is named `QR code 2`, `QR code 3`, and so on, and a new plate goes 5 mm
  to the right of what the design already holds, instead of failing or landing on the first one.
- The MIT licence, a demo GIF, and a security workflow (gitleaks, `cargo audit`, dependency review and
  actionlint) with Dependabot.

### Changed
- **Move away from the face** (`offset`) is replaced by Move Z, or by Move along > Face axes. A script that sends
  `offset` other than 0 now gets an error that says so.
- A very long text is refused earlier (about 100 characters of link) so a run stays under the 30 s a plugin may
  take, instead of being stopped half-way.
- The plugin also asks to run `appearance.assign` and `document.inspect`, so SolidCraft asks for an approval again.
- The tests now check the volume and the number of coloured faces, and cover a long link, a second code, a
  cylinder and a curved face.

### Fixed
- Running the plugin a second time in the same design failed with "another feature is called `QR code`".
- A code sunk into a face no longer colours the part's own top face.

## [0.4.0] - 2026-11-23 (not released)

### Changed
- The code is drawn as one sketch and one extrusion of the outline of the dark modules, instead of many boxes,
  so it stays an editable sketch with no extra lines.

## [0.3.0] - 2026-11-23 (not released)

### Changed
- The form is reduced to plain settings: text, face, style, size and height. The technical choices are fixed.

## [0.2.0] - 2026-11-23 (not released)

### Added
- The code can go on a flat face of your model, with an offset along its normal.
- The whole result stays in one body.

## [0.1.0] - 2026-11-23 (not released)

### Added
- First version: a plate carrying a QR code, raised or engraved.

[0.5.1]: ../../releases/tag/v0.5.1
[0.5.0]: ../../releases/tag/v0.5.0
