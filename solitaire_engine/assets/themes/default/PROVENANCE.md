# Default theme — provenance

This directory is the bundled-default card theme that ships embedded in
the binary via Bevy's `embedded_asset!` macro (see
`solitaire_engine/src/assets/sources.rs`). At runtime its files are
addressable as `embedded://solitaire_engine/assets/themes/default/...`.

## Current state (Phase 3)

The `theme.ron` manifest in this directory lists all 52 face slots plus
a back slot, but **the referenced SVG files do not yet exist**. The
manifest is intentionally a stub so that:

1. `embedded_asset!` has a real file to bundle (the manifest itself).
2. `ThemeManifest::validate` accepts the manifest (it requires all 52
   faces to be listed by name).
3. The `embedded://` asset source can be source-registered and queried
   without runtime errors during Phase 3.

The actual SVG art will be added when the project swaps in the
`hayeah/playing-cards-assets` artwork — see the implementation plan in
`/CARD_PLAN.md`. At that point, every `.svg` filename listed in
`theme.ron`'s `faces` map (and `back.svg`) must be added here, and each
new file needs a corresponding `embedded_asset!(app, ...)` call in
`solitaire_engine/src/assets/sources.rs::register_default_theme`.

## How to add files to the bundled default theme

For each new file you drop into this directory:

1. Drop the file under `solitaire_engine/assets/themes/default/`.
2. Add one line to `register_default_theme` in
   `solitaire_engine/src/assets/sources.rs` of the form:
   ```rust
   embedded_asset!(app, "../../assets/themes/default/<filename>");
   ```
   (The path is relative to `sources.rs`, which lives in
   `solitaire_engine/src/assets/`.)
3. Update this file with the licence and origin of the new asset.

## Licence

To be filled in once real artwork lands.
