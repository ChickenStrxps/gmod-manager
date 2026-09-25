# GMod Manager

Manage Garry's Mod modpacks. Pick a preset, hit Play.

## Install

1. Download `GModManager-Windows.zip` from the [latest release](https://github.com/ChickenStrxps/gmod-manager/releases/latest).
2. Unzip it anywhere and run `gmod-manager.exe`.

You need Steam and Garry's Mod installed. The app updates itself.

## Use

- **Play** applies the preset and starts GMod. Missing Workshop mods download after GMod opens; stay on its main menu until **Addons → GMod Manager** shows no mods “not ready.” The Mods tab in this app reports how many Workshop copies GMod has mounted (library copies are separate). If you entered a map while downloads were still finishing, restart the map to load newly available scripts and models.
- **Mods** – find mods on the Workshop, see their authors and sizes. Discovery saves a mod to the preset immediately. **Check required mods** scans up to 10 uncached Workshop pages per press; successful checks are cached, and any required mods found are saved even if Steam then rate-limits the scan. Press again to continue from where it stopped. If Steam returns HTTP 429, wait before trying again. The app does not run this page-by-page check automatically on startup.
- **Binds / Settings** – keybinds and console settings that come with the preset.
- **Import** – paste a public Garry's Mod Steam collection link or ID to create a preset with its mods; use **Details → Update mods from collection** to refresh it. Or choose a shared preset ZIP/JSON.
- **… → Share as file** – send a preset ZIP to a friend; they open it through **Import**.
- **… → Undo last apply** – puts your GMod files back.

## Library

Turn it on in **Library**. It keeps a compressed copy of every mod, so switching presets doesn't mean downloading everything again. Mods you aren't using get removed from Steam, and they come back from the library when you need them.

In GMod, these mods show up under **Addons → GMod Manager** instead of **Subscribed**.

In the spawn menu, **Entities** only shows entity addons. Look under **Weapons** for weapons and **Browse → Addons** for models and spawnlists; an addon does not necessarily add anything to Entities.

When addon syncing is on, **Play** also installs a small client-side spawn-menu fallback. Existing weapon and entity icons stay unchanged. If an addon supplies no icon under its class name, GMod Manager tries a related icon, then its model, then a standard category symbol; this does not repair missing game models or textures. Re-apply and restart GMod after updating the manager to load this change.

## Building

```powershell
cargo test
.\build.ps1
```

To publish an update, bump `version` in `Cargo.toml`, commit, and run `.\release.ps1`.
