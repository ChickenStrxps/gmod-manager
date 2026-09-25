# GMod Manager plan

## Goal

Build a Windows-first, portable desktop app that lets someone choose a modpack, edit its binds and addon settings, see exactly what will change, and apply it to their own Garry's Mod installation. A shared ZIP should contain the manager and profile definitions, not a copy of Garry's Mod or Workshop content.

## First preset: Lowgan's Portal Movement 3

- Video: https://www.youtube.com/watch?v=JGETrLfZJrY
- Collection linked in the video description: https://steamcommunity.com/sharedfiles/filedetails/?id=3428047388
- `presets/lowgan-portal-movement-3.json` is a snapshot taken on 2026-09-24: 93 Workshop IDs, 90 currently available and 3 unavailable. The manager should show the three unavailable IDs as unresolved rather than silently substitute something else.
- The preset should offer **Open collection / Subscribe to all** and compare the current live collection with the saved snapshot. A refresh is an explicit user action so the pack does not silently change.
- The video also links a separate Google Drive folder with edited addons: https://drive.google.com/drive/folders/1lUB93u6_lvblchJLRV2HzM3Cm0fVFaaH . These are optional imports. The description says its edits to Beatrun, ArcCW, MW2 weapons, sword, grappling hook, portal skin, and Seamless Portals differ from Workshop versions. The portal skin was described as incomplete. Do not bundle these files in the manager. If a user imports an edited addon, flag the equivalent Workshop addon for disabling to avoid loading both.
- The collection is the authoritative starting point for membership. The video description does not provide a complete bind or ConVar list, so do not invent author settings. The user can set and share their own profile values.

## User flow

1. Unzip and run the app. Detect Steam libraries and app ID 4000; allow manual game-path selection. Confirm the selected directory has `garrysmod/cfg`.
2. Select a profile or import a shared profile ZIP/JSON. Show Workshop availability, subscription/download state where locally detectable, missing custom files, bind conflicts, and file changes.
3. Edit binds in a key/command table, edit known ConVars in typed fields, and edit per-addon files in a raw text view when no structured adapter exists. Preserve unknown lines and comments.
4. Preview changes, then **Apply** while GMod is closed. Back up every touched file, write changes atomically, and retain a one-click **Restore previous state** action.
5. Launch through Steam or open the game manually. The app reports anything still requiring an in-game step, especially Workshop subscription or a client-side behavior check.

## Architecture

- **App:** Rust native executable with egui/eframe for a compact portable ZIP. Keep profiles and backups beside the executable; store only the chosen local game path in local app state. Ship no background service or installer.
- **Profile format:** versioned JSON with Workshop IDs, optional custom-addon references, bind definitions, ConVars, and explicit file mappings. Use relative paths within a profile; never bake in a friend's absolute path.
- **Workshop layer:** read public collection metadata; identify IDs, titles, unavailable items, and changes. Steam/GMod remains responsible for downloading subscribed Workshop items. Never request Steam credentials.
- **Addon state:** prefer GMod's native addon preset (`garrysmod/settings/addonpresets.txt`) as the compatibility baseline. Research and test `garrysmod/cfg/addonnomount.txt` against the installed GMod version before using it for direct external apply. If its format or behavior cannot be verified, generate/import the native preset and guide the user through loading it in GMod. Provide additive mode by default; an explicit isolated mode can disable addons outside the pack.
- **Bindings:** keep profile-specific commands in a manager-owned CFG and integrate them with the game's startup config without replacing unrelated user commands. Check execution order and persistence in a real client. Validate duplicate keys and escape command text.
- **Addon configs:** support ConVars and declared file mappings first. Add typed adapters only for addons whose config format and location are confirmed. Never assume all addon options live in one file.
- **Custom addon imports:** copy only user-supplied files into a manager-owned addon directory or reversible overlay. Show a diff and conflict warning before installation. Track hashes and original files for restore.
- **Mod library (0.5):** optional zstd-compressed store of Steam-downloaded GMAs (plain `.gma` or LZMA `_legacy.bin`). The preset's mods that Steam no longer holds are unpacked into `addons/gmm_<id>/`. Stored mods that aren't in the active preset are unsubscribed once through the menu bridge. Only 12 of the 93 Lowgan items expose an anonymous `file_url`, so Steam stays the only downloader.

## Build order and acceptance checks

1. **Discovery and snapshot:** locate GMod, parse Steam library metadata, import a collection by ID, and save/compare its member list. Check the 93-entry Lowgan snapshot and the three unavailable IDs.
2. **Profile editor:** create/clone/export/import profiles; edit binds, ConVars, and mapped files; validate schema, paths, duplicate IDs, and unsafe destinations.
3. **Apply and restore:** preview exact changes; require the game to be closed; back up touched files; perform atomic writes; restore the prior state. Verify repeated apply is idempotent and a failed write rolls back.
4. **Addon activation:** merge a native addon preset without overwriting existing presets. Test direct activation against an actual GMod installation and keep the native preset path as fallback.
5. **Lowgan preset:** display all available Workshop members and unresolved entries, open the collection for subscription, and show the optional edited-addon import instructions. Verify the snapshot can be exported and imported on a clean machine with path selection.
6. **Release:** build a Windows ZIP, run on a second Windows user profile, and check that no absolute paths or personal config files are included.

## Testing strategy

- Use fixture files for CFG parsing, JSON validation, Steam library discovery, path safety, backup/restore, and addon preset merging.
- Use a copy of `garrysmod` config files for apply/restore tests before touching the installed game.
- A GMod dedicated server can run without a graphical client and load a Workshop collection, which is useful for server-side addon errors. It cannot verify client binds, camera behavior, or movement feel. Those need a short real-client smoke test. There is no reason to launch the game during planning.

## Source notes

- Facepunch addon installation: https://wiki.facepunch.com/gmod/Addons
- Facepunch addon preset file location: https://wiki.facepunch.com/gmod/Global.LoadAddonPresets
- Facepunch addon menu behavior: https://wiki.facepunch.com/gmod/Addons_Menu
- Facepunch Steamworks addon mounting API: https://wiki.facepunch.com/gmod/steamworks
- Facepunch dedicated server and Workshop collection instructions: https://wiki.facepunch.com/gmod/Downloading_a_Dedicated_Server and https://wiki.facepunch.com/gmod/Workshop_for_Dedicated_Servers
- egui native framework: https://github.com/emilk/egui
