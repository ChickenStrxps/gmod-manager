"""Speaks each scene's narration with a Microsoft neural voice (edge-tts).

Writes promo/public/voice/<scene>.mp3 and voice.json with each clip's length.
Unchanged lines keep their existing audio, so re-rendering is quick.
"""

import asyncio
import hashlib
import json
import subprocess
from pathlib import Path

import edge_tts

PROMO = Path(__file__).resolve().parents[1]
OUT = PROMO / "public" / "voice"


def seconds(path: Path) -> float:
    result = subprocess.run(
        ["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", str(path)],
        capture_output=True, text=True, check=True,
    )
    return float(result.stdout.strip())


async def main() -> None:
    script = json.loads((PROMO / "script.json").read_text(encoding="utf-8"))
    OUT.mkdir(parents=True, exist_ok=True)
    index_path = OUT / "voice.json"
    old = json.loads(index_path.read_text(encoding="utf-8")) if index_path.exists() else {}
    index = {}
    for scene in script["scenes"]:
        key = "|".join([script["voice"], script["rate"], script["pitch"], scene["narration"]])
        digest = hashlib.sha1(key.encode()).hexdigest()
        mp3 = OUT / f"{scene['id']}.mp3"
        cached = old.get(scene["id"], {})
        if not (mp3.exists() and cached.get("hash") == digest):
            speech = edge_tts.Communicate(
                scene["narration"], script["voice"], rate=script["rate"], pitch=script["pitch"]
            )
            await speech.save(str(mp3))
            print(f"voice: {scene['id']}")
        index[scene["id"]] = {"hash": digest, "seconds": seconds(mp3)}
    index_path.write_text(json.dumps(index, indent=2), encoding="utf-8")


if __name__ == "__main__":
    asyncio.run(main())
