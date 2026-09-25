"""Lays out the video: scene timing, narration, clips and sound cues.

Reads script.json, voice.json, capture.json and the version in Cargo.toml, and
writes src/manifest.generated.json, which is all the Remotion side needs.
Prints the total length so the music can be made to fit.
"""

import json
import re
from pathlib import Path

PROMO = Path(__file__).resolve().parents[1]
ROOT = PROMO.parent
FPS = 50
CROSSFADE = 0.5
VOICE_AT = 0.5
TITLE_TAIL = 1.1
SFX = {
    "click": ("sfx/click.ogg", 0.55),
    "key": ("sfx/key.ogg", 0.22),
    "scroll": ("sfx/scroll.ogg", 0.12),
}


def frames(seconds: float) -> int:
    return round(seconds * FPS)


def version() -> str:
    cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'^version\s*=\s*"([^"]+)"', cargo, re.MULTILINE)
    if not match:
        raise SystemExit("No version in Cargo.toml")
    return match.group(1)


def main() -> None:
    script = json.loads((PROMO / "script.json").read_text(encoding="utf-8"))
    voice = json.loads((PROMO / "public/voice/voice.json").read_text(encoding="utf-8"))
    capture = json.loads((PROMO / "public/capture/capture.json").read_text(encoding="utf-8"))
    scenes = []
    start = 0.0
    for index, scene in enumerate(script["scenes"]):
        spoken = voice[scene["id"]]["seconds"]
        sfx = []
        if scene["kind"] == "capture":
            clip = capture[scene["id"]]
            length = clip["seconds"]
            for event in clip["events"]:
                src, volume = SFX[event["kind"]]
                sfx.append({"src": src, "at": frames(event["t"]), "volume": volume})
        else:
            extra = 0.6 if index == 0 else 0.0
            length = VOICE_AT + extra + spoken + TITLE_TAIL
            sfx.append({"src": "sfx/chime.ogg", "at": frames(0.25), "volume": 0.35})
        if index > 0:
            sfx.append({"src": "sfx/whoosh.ogg", "at": 0, "volume": 0.3})
        scenes.append(
            {
                "id": scene["id"],
                "kind": scene["kind"],
                "headline": scene["headline"],
                "sub": scene["sub"],
                "from": frames(start),
                "duration": frames(length),
                "voice": {"src": f"voice/{scene['id']}.mp3", "at": frames(VOICE_AT + (0.6 if index == 0 else 0.0))},
                "voiceFrames": frames(spoken),
                "clip": f"capture/{scene['id']}.mp4" if scene["kind"] == "capture" else None,
                # Optional push-in on part of the clip: centre (0-1), zoom, and when.
                "focus": {
                    "x": scene["focus"]["x"],
                    "y": scene["focus"]["y"],
                    "zoom": scene["focus"]["zoom"],
                    "from": frames(scene["focus"]["from"]),
                    "to": frames(scene["focus"]["to"]),
                }
                if "focus" in scene
                else None,
                "sfx": sfx,
            }
        )
        start += length - CROSSFADE
    total = frames(start + CROSSFADE)
    manifest = {
        "version": version(),
        "fps": FPS,
        "width": 1920,
        "height": 1080,
        "total": total,
        "crossfade": frames(CROSSFADE),
        "music": "music.wav",
        "scenes": scenes,
    }
    out = PROMO / "src" / "manifest.generated.json"
    out.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print(f"{total / FPS:.2f}")


if __name__ == "__main__":
    main()
