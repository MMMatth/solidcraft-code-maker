# Security policy

## Supported versions

Only the latest release gets fixes.

## Reporting a vulnerability

Please do not open a public issue for a security problem.

Use **Security > Report a vulnerability** on this repository's GitHub page to send a private report.
Say which version you use, what you did and what happened. I will answer as soon as I can and tell you
when a fix is released.

## What the plugin can do

The plugin runs in SolidCraft's sandbox. It asks for no file access and no network access. It only runs the
SolidCraft commands listed in `code-maker/plugin.json` (`solid.box`, `sketch.create`, `sketch.line`,
`sketch.finish`, `solid.extrude`, `model.faces`, `appearance.assign` and `document.inspect`). Check that
list when you install a new version.
