"""Records the raw app, scene by scene, in a sandbox with public Workshop data.

Needs the promo build (see demo.py) and voice.json from voice.py: each clip
runs at least as long as its narration. Runs in the background: the window is
off screen and driven through stdin, so you can keep using your PC.

Writes promo/public/capture/<scene>.mp4 and capture.json with each clip's
length and the moments of clicks, keys and scrolls (seconds into the clip).
"""

import ctypes
import json
import time
from pathlib import Path

import demo

PROMO = Path(__file__).resolve().parents[1]
OUT = PROMO / "public" / "capture"

# Public collections: one set up off camera, one imported on camera.
SEEDED = "https://steamcommunity.com/sharedfiles/filedetails/?id=2798594136"
IMPORTED = "https://steamcommunity.com/sharedfiles/filedetails/?id=2185604753"

# Where things sit, in egui points at the capture size (negative = from right/bottom).
IMPORT = (173, 122)
PRESET = [(116, 169), (116, 219), (116, 269)]  # built-in first, then in order of import
TAB_MODS, TAB_BINDS, TAB_SETTINGS = (278, 194), (363, 194), (445, 194)
ADD_MODS = (-81, 242)
DONE = (-57, 242)
SEARCH_BOX = (500, 296)
RESULT_ROWS = [355, 402, 446, 493]
RESULT_ADD_X = -87
PLAY = (-156, 137)
MORE = (-68, 137)
UNDO_ITEM = (-197, 236)
LIBRARY_NAV = (40, -150)
LIBRARY_TOGGLE = (278, 112)
LIST = (640, 450)
IMPORT_BUTTON = (-157, 51.5)  # from the window centre


def hold_until(start: float, seconds: float) -> None:
    left = start + seconds - time.time()
    if left > 0:
        time.sleep(left)


def overview(app: demo.App) -> None:
    app.move(*LIST, 1.0)
    time.sleep(0.4)
    app.scroll(-110, steps=4, gap=0.35)
    time.sleep(0.5)
    app.move(700, 420, 0.7)
    time.sleep(0.6)
    app.scroll(110, steps=4, gap=0.25)
    time.sleep(0.3)
    app.click(*PRESET[0], 0.8)
    time.sleep(1.3)
    app.click(*TAB_BINDS)
    time.sleep(1.0)
    app.click(*TAB_SETTINGS, 0.4)
    time.sleep(0.8)
    app.click(*TAB_MODS, 0.5)
    time.sleep(0.8)
    app.click(*PRESET[1], 0.8)
    time.sleep(0.6)
    app.move(*LIST, 0.9)


def import_collection(app: demo.App) -> None:
    app.click(*IMPORT, 0.8)
    time.sleep(0.7)
    app.paste(IMPORTED)
    time.sleep(0.8)
    app.click(*app.centered(*IMPORT_BUTTON), 0.7)
    time.sleep(4.5)
    app.move(*LIST, 1.0)
    app.scroll(-110, steps=3, gap=0.35)


def discover(app: demo.App) -> None:
    app.click(*ADD_MODS, 0.8)
    time.sleep(0.4)
    app.click(*SEARCH_BOX, 0.6)
    time.sleep(0.2)
    app.type("portal")
    time.sleep(0.35)
    app.type(" gun")
    time.sleep(3.2)
    app.move(700, RESULT_ROWS[0], 0.6)
    time.sleep(0.3)
    app.move(700, RESULT_ROWS[1], 0.4)
    time.sleep(0.4)
    app.click(RESULT_ADD_X, RESULT_ROWS[1], 0.7)
    time.sleep(1.2)
    app.click(*DONE, 0.8)


def library(app: demo.App) -> None:
    app.click(*LIBRARY_NAV, 1.0)
    time.sleep(1.0)
    app.click(*LIBRARY_TOGGLE, 0.9)
    time.sleep(0.8)
    app.move(500, 200, 1.0)


def play(app: demo.App) -> None:
    app.click(*PRESET[1], 0.9)
    time.sleep(1.2)
    app.move(*PLAY, 0.9)
    time.sleep(1.6)
    app.click(*MORE, 0.5)
    time.sleep(0.5)
    app.move(*UNDO_ITEM, 0.6)
    time.sleep(1.4)
    app.press("escape")
    time.sleep(0.3)
    app.move(*PLAY, 0.6)


SCENES = {
    "overview": overview,
    "import": import_collection,
    "discover": discover,
    "library": library,
    "play": play,
}


def main() -> None:
    voice = json.loads((PROMO / "public" / "voice" / "voice.json").read_text(encoding="utf-8"))
    script = json.loads((PROMO / "script.json").read_text(encoding="utf-8"))
    # Windows stops drawing windows while the display sleeps, which would record black.
    ES_CONTINUOUS, ES_DISPLAY_REQUIRED = 0x80000000, 0x00000002
    ctypes.windll.kernel32.SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED)
    demo.prepare()
    app = demo.App(1600, 1000, zoom=1.21)
    index = {}
    try:
        # Off camera: a second preset so there is something to switch between.
        app.click(*IMPORT, 0.1)
        time.sleep(0.5)
        app.paste(SEEDED)
        time.sleep(0.2)
        app.click(*app.centered(*IMPORT_BUTTON), 0.1)
        time.sleep(9)
        for scene in script["scenes"]:
            if scene["kind"] != "capture":
                continue
            out = OUT / f"{scene['id']}.mp4"
            app.events.clear()
            recorder = demo.Recorder(app, out)
            SCENES[scene["id"]](app)
            # Room for the narration, which starts a beat into the scene.
            hold_until(recorder.started, voice[scene["id"]]["seconds"] + 1.6)
            seconds = recorder.stop()
            index[scene["id"]] = {
                "seconds": seconds,
                "events": [
                    {"t": round(t - recorder.started, 3), "kind": kind} for t, kind in app.events
                ],
            }
            print(f"capture: {scene['id']} {seconds:.1f}s")
    finally:
        app.close()
    (OUT / "capture.json").write_text(json.dumps(index, indent=2), encoding="utf-8")


if __name__ == "__main__":
    main()
