# GMod Manager

Manage Garry's Mod modpacks. Pick a preset, hit Play.

## Install

1. Download `GModManager-Windows.zip` from the [latest release](https://github.com/ChickenStrxps/gmod-manager/releases/latest).
2. Unzip it anywhere and run `gmod-manager.exe`.

You need Steam and Garry's Mod installed. The app updates itself.

## Use

- **Play** applies the preset and starts GMod. Missing mods download when GMod opens.
- **Mods** – find mods on the Workshop, see their authors and sizes. The app checks existing presets for required mods too; any missing ones download on Play. Use **Check required mods** to retry if Steam was offline.
- **Binds / Settings** – keybinds and console settings that come with the preset.
- **… → Share as file** – send a preset to a friend. They add it with **Import**.
- **… → Undo last apply** – puts your GMod files back.

## Library

Turn it on in **Library**. It keeps a compressed copy of every mod, so switching presets doesn't mean downloading everything again. Mods you aren't using get removed from Steam, and they come back from the library when you need them.

In GMod, these mods show up under **Addons → GMod Manager** instead of **Subscribed**.

## Building

```powershell
cargo test
.\build.ps1
```

To publish an update, bump `version` in `Cargo.toml`, commit, and run `.\release.ps1`.
