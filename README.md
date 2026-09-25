# GMod Manager

Manage Garry's Mod modpacks. Pick a preset, hit Play.

## Preview

[![Finding mods as you type in GMod Manager](assets/intro-preview.gif)](assets/gmod-manager-intro.mp4)

Watch the [full intro video](assets/gmod-manager-intro.mp4) (about a minute, with sound).

## Install

1. Download `GModManager-Windows.zip` from the [latest release](https://github.com/ChickenStrxps/gmod-manager/releases/latest).
2. Unzip it anywhere and run `gmod-manager.exe`.

You need Steam and Garry's Mod installed. The app updates itself.

## Use

- **Play** applies the preset and starts GMod. Missing Workshop mods download after GMod opens; stay on its main menu until **Addons → GMod Manager** shows no mods “not ready.” The Mods tab in this app reports how many Workshop copies GMod has mounted (library copies are separate). If you entered a map while downloads were still finishing, restart the map to load newly available scripts and models.
- **Mods** – find mods on the Workshop, see their authors and sizes. Results update as you type. Switch between **List** and **Grid**, and use the status chips (installed, in library, to download, removed) to narrow the list. Discovery saves a mod to the preset immediately. **Check required mods** now asks your running Steam client for dependencies in batches using GMod's installed Steam runtime. Successful checks are cached, including nested dependencies. If Steam queries fail, use **Settings → Required mods → Workshop pages (fallback)** to use the older page-by-page scanner. That mode checks up to 10 uncached pages per press and may get HTTP 429; successful pages and discovered mods are saved for the next press. Both modes run only when you press the button.
- **Binds / Settings** – keybinds and console settings that come with the preset.
- **Import** – paste a public Garry's Mod Steam collection link or ID to create a preset with its mods; use **Details → Update mods from collection** to refresh it. Or choose a shared preset ZIP/JSON.
- **… → Share as file** – send a preset ZIP to a friend; they open it through **Import**.
- **… → Undo last apply** – puts your GMod files back.
- **Ctrl+K** (or **Search** in the sidebar) – jump to a preset, tab or action like Play, Review or Share from the keyboard.

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

## Intro video

`.\promo\render.ps1` films the app and renders the intro for the version in `Cargo.toml`, with that version in the corner. It records a sandboxed copy in `C:\GModManagerDemo` with public Workshop collections, off screen, so none of your presets or files appear and you can keep using the PC. Edit the narration and captions in `promo/script.json`; `-SkipCapture` re-renders without filming again. The voice is a Microsoft neural voice via `edge-tts`; the sounds are CC0 from Kenney (`promo/public/sfx`).
