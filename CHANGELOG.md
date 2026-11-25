# Changelog

All notable changes to the plugin. The format follows [Keep a Changelog](https://keepachangelog.com/),
and the versions follow [Semantic Versioning](https://semver.org/).

Version 0.5.0 is the first published release. Versions 0.1.0 to 0.4.0 were development steps of the first
day, kept below for the history: they have no tag and no release.

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

[0.5.0]: ../../releases/tag/v0.5.0
