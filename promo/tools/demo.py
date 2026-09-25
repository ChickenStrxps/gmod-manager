"""Runs GMod Manager in a throwaway sandbox and records it without touching your PC.

The promo build (`cargo build --release --features promo --target-dir target/promo`)
takes scripted mouse and keyboard input on stdin and paints its own cursor, and
the window sits off screen, so your real mouse, keyboard and focus are never used.
Frames are copied with PrintWindow, which works for windows nobody can see.

The release build keeps its presets, library and caches beside the exe, so a
copy in the sandbox starts with nothing of yours: only the built-in preset,
plus whatever the capture script imports from public Workshop collections.
Its Garry's Mod folder is an empty stand-in, so nothing touches your real game.
The sandbox sits at `C:\\GModManagerDemo` (override with GMM_DEMO_DIR) because
the app shows its folders on screen, and a path under your profile would show
your user name.
"""

from __future__ import annotations

import ctypes
import json
import os
import random
import shutil
import subprocess
import threading
import time
from pathlib import Path

import win32con
import win32gui
import win32process
import win32ui

# Physical pixels everywhere, whatever the display scaling.
ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))

ROOT = Path(__file__).resolve().parents[2]
DEMO = Path(os.environ.get("GMM_DEMO_DIR", r"C:\GModManagerDemo"))
EXE = ROOT / "target" / "promo" / "release" / "gmod-manager.exe"
# Far right of any monitor, so the window is never in anyone's way.
OFF_SCREEN = (12000, 0)
PW_CLIENTONLY_RENDERFULLCONTENT = 3


def prepare() -> None:
    """Fresh sandbox: the promo exe, an empty stand-in GMod folder, default state."""
    if not EXE.exists():
        raise SystemExit(f"Build the promo exe first: {EXE} is missing")
    if DEMO.exists():
        shutil.rmtree(DEMO)
    game = DEMO / "GarrysMod" / "garrysmod"
    for sub in ("cfg", "addons", "lua/menu"):
        (game / sub).mkdir(parents=True)
    shutil.copy2(EXE, DEMO / EXE.name)
    (DEMO / "state.json").write_text(json.dumps({"gamePath": str(game)}), encoding="utf-8")


class App:
    """The sandboxed app, off screen at a fixed size, driven through stdin."""

    def __init__(self, width: int, height: int, zoom: float):
        # Every input is logged with a wall-clock time so the video can put sounds on it.
        self.events: list[tuple[float, str]] = []
        self.zoom = zoom
        self.proc = subprocess.Popen([str(DEMO / EXE.name)], cwd=DEMO, stdin=subprocess.PIPE)
        self.hwnd = self._find_window()
        self._size(width, height)
        self.send(f"zoom {zoom}")
        self.cursor = (self.logical[0] * 0.6, self.logical[1] * 0.55)
        self.send(f"move {self.cursor[0]} {self.cursor[1]} 0")
        time.sleep(1.0)

    def _find_window(self) -> int:
        deadline = time.time() + 20
        while time.time() < deadline:
            found = []

            def visit(hwnd, _):
                if win32gui.IsWindowVisible(hwnd):
                    if win32process.GetWindowThreadProcessId(hwnd)[1] == self.proc.pid:
                        found.append(hwnd)

            win32gui.EnumWindows(visit, None)
            if found:
                return found[0]
            time.sleep(0.2)
        raise SystemExit("GMod Manager window never appeared")

    def _size(self, width: int, height: int) -> None:
        """Moves the window off screen with a client area of exactly width x height."""
        flags = win32con.SWP_NOACTIVATE | win32con.SWP_NOZORDER
        for _ in range(3):
            left, top, right, bottom = win32gui.GetWindowRect(self.hwnd)
            cw, ch = self.client_size()
            win32gui.SetWindowPos(
                self.hwnd, 0, *OFF_SCREEN,
                width + (right - left) - cw, height + (bottom - top) - ch, flags,
            )
            time.sleep(0.4)
            if self.client_size() == (width, height):
                return
        raise SystemExit(f"Could not size the window to {width}x{height}")

    def client_size(self) -> tuple[int, int]:
        _, _, right, bottom = win32gui.GetClientRect(self.hwnd)
        return right, bottom

    @property
    def logical(self) -> tuple[float, float]:
        """Client size in egui points."""
        w, h = self.client_size()
        return w / self.zoom, h / self.zoom

    def _point(self, x: float, y: float) -> tuple[float, float]:
        """Negative values count from the right/bottom edge."""
        lw, lh = self.logical
        return (x + lw if x < 0 else x), (y + lh if y < 0 else y)

    def centered(self, dx: float, dy: float) -> tuple[float, float]:
        """egui point offset from the middle of the window, where modals open."""
        lw, lh = self.logical
        return lw / 2 + dx, lh / 2 + dy

    def send(self, line: str) -> None:
        self.proc.stdin.write((line + "\n").encode())
        self.proc.stdin.flush()

    def _log(self, kind: str) -> None:
        self.events.append((time.time(), kind))

    def move(self, x: float, y: float, duration: float = 0.55) -> None:
        x, y = self._point(x, y)
        self.send(f"move {x:.1f} {y:.1f} {duration}")
        self.cursor = (x, y)
        time.sleep(duration)

    def click(self, x: float, y: float, duration: float = 0.55, settle: float = 0.12) -> None:
        self.move(x, y, duration)
        time.sleep(settle)
        self._log("click")
        self.send("click")
        time.sleep(0.05)

    def type(self, text: str, cps: float = 11.0) -> None:
        rng = random.Random(text)
        for char in text:
            self._log("key")
            self.send(f"text {char}")
            # People type in bursts, not on a metronome.
            time.sleep(rng.uniform(0.6, 1.5) / cps)

    def paste(self, text: str) -> None:
        self._log("key")
        self.send(f"paste {text}")

    def press(self, key: str) -> None:
        self._log("key")
        self.send(f"key {key}")

    def scroll(self, dy: float, steps: int = 1, gap: float = 0.25) -> None:
        for _ in range(steps):
            self._log("scroll")
            self.send(f"scroll {dy}")
            time.sleep(gap)

    def close(self) -> None:
        self.proc.terminate()
        try:
            self.proc.wait(5)
        except subprocess.TimeoutExpired:
            self.proc.kill()


class Grabber:
    """Copies a window's client area with PrintWindow, reusing one bitmap."""

    def __init__(self, hwnd: int, width: int, height: int):
        self.hwnd, self.size = hwnd, (width, height)
        self.window_dc = win32gui.GetWindowDC(hwnd)
        self.src = win32ui.CreateDCFromHandle(self.window_dc)
        self.mem = self.src.CreateCompatibleDC()
        self.bitmap = win32ui.CreateBitmap()
        self.bitmap.CreateCompatibleBitmap(self.src, width, height)
        self.mem.SelectObject(self.bitmap)

    def grab(self) -> bytes:
        ctypes.windll.user32.PrintWindow(
            ctypes.c_void_p(self.hwnd),
            ctypes.c_void_p(self.mem.GetSafeHdc()),
            PW_CLIENTONLY_RENDERFULLCONTENT,
        )
        return self.bitmap.GetBitmapBits(True)

    def close(self) -> None:
        win32gui.DeleteObject(self.bitmap.GetHandle())
        self.mem.DeleteDC()
        self.src.DeleteDC()
        win32gui.ReleaseDC(self.hwnd, self.window_dc)


class Recorder:
    """Records the app window to an MP4 at a constant frame rate until stopped."""

    def __init__(self, app: App, out: Path, fps: int = 50):
        width, height = app.client_size()
        out.parent.mkdir(parents=True, exist_ok=True)
        self.fps = fps
        self.grabber = Grabber(app.hwnd, width, height)
        self.ffmpeg = subprocess.Popen(
            [
                "ffmpeg", "-y", "-loglevel", "error",
                "-f", "rawvideo", "-pix_fmt", "bgra", "-s", f"{width}x{height}",
                "-r", str(fps), "-i", "-",
                "-c:v", "libx264", "-preset", "veryfast", "-crf", "12",
                "-pix_fmt", "yuv420p", str(out),
            ],
            stdin=subprocess.PIPE,
        )
        self.frames = 0
        self.blank = 0
        self.stopping = False
        self.started = time.time()
        self.thread = threading.Thread(target=self._run, daemon=True)
        self.thread.start()

    def _run(self) -> None:
        frame = b""
        while not self.stopping:
            due = self.started + self.frames / self.fps
            wait = due - time.time()
            if wait > 0:
                time.sleep(wait)
            # When behind, repeat the last frame so the clock in the video stays true.
            if not frame or time.time() - due < 1 / self.fps:
                frame = self.grabber.grab()
                # A blue-channel sample of all zeros means Windows isn't drawing the window.
                if not any(frame[::40_004]):
                    self.blank += 1
            self.ffmpeg.stdin.write(frame)
            self.frames += 1

    def stop(self) -> float:
        """Ends the clip and returns its length in seconds."""
        self.stopping = True
        self.thread.join()
        self.ffmpeg.stdin.close()
        self.ffmpeg.wait(60)
        self.grabber.close()
        if self.blank > self.frames // 5:
            raise SystemExit(
                "Recorded blank frames: Windows stops drawing windows while the display is off "
                "or the PC is locked. Keep the screen on and run the capture again."
            )
        return self.frames / self.fps
