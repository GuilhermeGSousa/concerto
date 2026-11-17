# concerto-texture-editor

The asset editor of a `Texture`, as an extension of `concerto-editor`. Only
editor crates depend on it. A game lists it in its own `Cargo.toml`, as
`cargo concerto new` does, and `cargo concerto editor <dir> --update` writes
the call into the game's generated `editor/`:

```toml
[package.metadata.concerto.editor]
plugins = [{ package = "concerto-texture-editor", install = "concerto_texture_editor::install" }]
```

`install` registers `TextureEditorPlugin`, which registers `TextureEditor` for
`Texture` with `register_asset_editor`. A texture is not creatable: textures
come from importing an image or a glTF. Without this crate a texture has no
editor and opening one opens no tab.

## The tab

Opening a texture, by double-clicking it in the Asset Browser or with MCP
`open_asset`, reads its `.gasset`: the payload, and the source image, which is
the file's editor-only section. A texture whose file holds no source image,
such as one written before the source moved to that section, fails to open,
saying to import its source again.

- **Preview.** The centre shows the texture's derived levels as the GPU samples
  them, block-compression artefacts included, scaled to fit 512 pixels, over
  the status line.
- **Status line.** `<width>×<height> · <format> · <levels> mips · <size> MB on
  the GPU`, for example `512×512 · Bc7RgbaUnormSrgb · 10 mips · 0.3 MB on the
  GPU`, where the size counts every level in MiB; or `Derive failed: <error>`,
  such as the refusal of sRGB with BC4 or BC5. MCP `status` shows it on the
  document's line.
- **Inspector.** One value, `Texture`, a `TextureDocument` holding the
  texture's `settings` (compression, colour space, max size, mips) and its
  `role`. The role is set by the importer: an edit that changes it is refused
  with "A texture's role is set by the importer and stays <role>; import its
  source again to change it".

An edit to the settings marks the tab unsaved and derives the levels again on a
worker thread, from the source the tab read from the file's editor-only
section when it opened, through the
project's derived-data cache, so a later load of the same settings hits the
cache. While it runs the document carries `BackgroundWork` and MCP `status`
reads `busy: yes`. A result for settings that changed again while it ran is
dropped.

Save, with Ctrl+S, the unsaved-changes prompt or MCP `save`, reads the file's
payload again, replaces only its settings, and writes it with
`resave_content_asset`, keeping its id and import provenance and copying its
editor-only section, so the source stays as it was. It ends
in `SceneSaved` naming the texture, so open scenes and previews reload it, and
the status reads `Saved <address>` until the next derive. A save requested
while an import is running or queued waits for the imports to finish, so it
never writes an older source over a newer one, and one requested while a
derive is running waits for it. While the last derive failed, save is refused
with `Save failed: Derive failed: <error>; change the settings until the
texture derives, then save`, which MCP `save` answers as `save_failed`, so no
file holds settings its texture cannot derive in. Discard restores the settings as
last read or saved.

A re-import keeps the saved settings, as for every texture; see
[asset-importing.md](../../docs/asset-importing.md).

MCP needs no tool of its own: `open_asset`, `inspect` and `set_property` with
`target: "Texture"` and paths such as `settings.compression` (`"Bc7"`) or
`settings.max_size` (`"S512"`), `save`, `status`, and `screenshot` of the
preview.
