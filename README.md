# GMod Manager

A portable Windows manager for Garry's Mod presets. Presets, backups, and caches live beside the executable, and each person's GMod installation is detected separately.

## Run

1. Download `GModManager-Windows.zip` from the [latest release](https://github.com/ChickenStrxps/gmod-manager/releases/latest) and unzip it anywhere writable.
2. Run `gmod-manager.exe`. Steam and Garry's Mod must already be installed.
3. Pick a preset in the sidebar and press **Play**. The preset is applied and GMod starts through Steam. Keep Steam online; new mods subscribe from GMod's main menu and may take time to download.

The app checks for a new release each time it starts. When one exists, **Update to x.y.z** appears in the sidebar. It downloads the new version, checks its SHA-256, swaps it in, and restarts. Presets, the library, and backups stay where they are. **Settings → Check for updates** checks on demand.

The preset screen has four tabs:

- **Mods**: every mod with its icon, Workshop size, and whether it's installed, in the library, or still to download. The header shows the preset's total size. **Add mods** searches the Workshop; you can also paste a link or ID. Right-click a mod to open it on the Workshop or remove it.
- **Binds**: **Copy from GMod** reads your current binds; untick any you don't want to share.
- **Settings**: console variables and mod config files under `cfg/`, `data/`, or `settings/`. Each row has its own tick box.
- **Details**: name, description, links, the Workshop collection ID (**Update mods from collection** pulls its current contents; mods you added yourself stay), and what gets applied.

Edits save automatically. The **…** menu has **Apply without launching**, **Review changes…**, **Undo last apply**, **Share as file…**, and **Duplicate**. A shared file is a ZIP containing the whole preset (Workshop IDs, links, binds, settings, config contents, and tick boxes). Friends open it with **Import** in the sidebar. Your game path, library, and backups stay out of the ZIP. Older preset JSON files can still be imported.

The bundled Lowgan preset has 93 IDs in its snapshot; 3 were taken down from the Workshop and are skipped. It includes the only bind the creator published: `h` → `use portal_gun`, which pulls out the Seamless Portals gun. The creator's video doesn't list any other binds or settings. Change the key on the **Binds** tab.

## Mod library

**Library** in the sidebar keeps one zstd-compressed copy of each mod, so switching presets doesn't redownload anything and mods you aren't using don't sit on disk at full size. On this machine's Lowgan pack, compressed copies were about 35–57% of the original size.

With **Use the library** on, **Play** does the following:

1. Copies each mod Steam has already downloaded for the preset into the library, once per Workshop update.
2. Unpacks mods from the library into `garrysmod/addons/gmm_<id>/` if they're in the preset but Steam no longer has them. GMod loads these as normal folder addons.
3. Removes unpacked mods that aren't in the preset. They stay in the library.
4. At the next GMod start, unsubscribes Steam from mods that are safely stored in the library but aren't in the preset. Steam then deletes its full-size copy.

A mod is only downloaded again when its Workshop page has a newer update than the stored copy. **Save installed mods now** compresses everything Steam already has without applying anything. The library folder defaults to `library/` beside the app. Use **Change…** to move it; the app warns if the folder is synced to OneDrive. With the library off, Steam keeps every mod at full size, as it did before.

## How applying works

- Mods served by Steam go into GMod's native `garrysmod/settings/addonpresets.txt` as `GMM · <name>`. The manager also removes the preset's IDs from `cfg/addonnomount.txt` if they are listed there as disabled.
- A marked include in `garrysmod/lua/menu/menu.lua` loads `garrysmod/lua/menu/gmm_workshop.lua`. On the next GMod launch, that script subscribes to missing items, enables them, and does the one-time library unsubscribe step. GMod writes progress to `garrysmod/data/gmm_workshop_status.json`, which appears next to the mod filter. Steam updates to GMod may replace the include; apply again if subscriptions stop working.
- The same script adds a **GMod Manager** entry under **Subscribed** in GMod's Addons menu. It shows every mod in the applied preset with its icon, size, and whether it came from the library or Steam. Library mods are loaded as folder addons, so GMod's own **Subscribed** list doesn't show them. Changes are made in the app ("Modify in GMM app").
- Ticked binds and ConVars are written to `cfg/gmm_active.cfg`. A small managed block in `cfg/autoexec.cfg` executes it on startup. Text outside that block is kept.
- Before writing, each apply backs up the files it changes to `backups/` beside the app and records which library mods it unpacked or removed. **Undo last apply** restores those files, removes mods that apply unpacked, and unpacks mods it removed. Apply refuses to run while GMod is open or if a file changed after the plan was made.

The creator's optional edited addons are linked from the preset but aren't bundled or installed by this app. The video description says the portal skin edit was incomplete. If you use edited versions, disable the conflicting Workshop versions in GMod.

Workshop search reads Steam's public search page and item details, so it needs an internet connection. Details and icons are cached in `cache/` beside the app.

## Build

Install the Rust toolchain for Windows (MSVC), then run:

```powershell
cargo test
.\build.ps1
```

The release archive is written to `dist/GModManager-Windows.zip`. It contains only the executable, the README, and the preset JSON.

## Release

1. Bump `version` in `Cargo.toml` and commit.
2. Run `.\release.ps1` (optionally `-Notes "what changed"`).

The script runs the tests, builds, tags `v<version>`, pushes, and creates a GitHub release with `gmod-manager.exe`, its `.sha256`, and the zip. The in-app updater reads the latest release from the repo named in `src/update.rs`. It uses the GitHub login Git Credential Manager has stored for the repo owner (`git credential-manager github login --username <owner>`).

## Sources

- Video: https://www.youtube.com/watch?v=JGETrLfZJrY
- Collection: https://steamcommunity.com/sharedfiles/filedetails/?id=3428047388
- GMod addon presets: https://wiki.facepunch.com/gmod/Global.LoadAddonPresets
- GMod addon controls: https://wiki.facepunch.com/gmod/Addons_Menu
- GMod folder addons: https://wiki.facepunch.com/gmod/Workshop_Addon_Creation
- GMod Steamworks subscriptions: https://wiki.facepunch.com/gmod/steamworks.Subscribe and https://wiki.facepunch.com/gmod/steamworks.Unsubscribe
- GMA format: https://github.com/Facepunch/gmad
- GMod menu state: https://wiki.facepunch.com/gmod/States
