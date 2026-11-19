"""Verified Win32 HWND video capture using Windows Graphics Capture (WGC).

Unlike FFmpeg gdigrab desktop/monitor or Ascent's DXGI monitor recordings,
WGC targets a specific native HWND, including when another window overlaps it.
Windows-only; requires "pip install --no-deps windows-capture" plus cv2/numpy.
No game/window focus, desktop input, microphone or hidden fallback to full-screen
recording. A clip fails validation rather than publishing unrelated windows.

CLI:
  py -B scripts/review_window_capture.py --hwnd 123456 --expected-pid 789 \
      --seconds 8 --output video.mkv [--reference game_screenshot.png]
"""
from __future__ import annotations

import argparse
import ctypes
from ctypes import wintypes
from dataclasses import asdict, dataclass
import json
import math
from pathlib import Path
import shutil
import subprocess
import threading
import time

import cv2
import numpy as np

try:
    from windows_capture import WindowsCapture
except ImportError as exc:
    raise SystemExit("Install WGC backend: py -m pip install --user --no-deps windows-capture") from exc

CREATE_NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0x08000000)
user32 = ctypes.windll.user32


def validate_hwnd(hwnd: int, expected_pid: int | None) -> dict:
    if hwnd <= 0 or not user32.IsWindow(wintypes.HWND(hwnd)):
        raise RuntimeError(f"Invalid or closed window HWND {hwnd}")
    if not user32.IsWindowVisible(wintypes.HWND(hwnd)):
        raise RuntimeError(f"Window HWND {hwnd} is not visible")
    if user32.IsIconic(wintypes.HWND(hwnd)):
        raise RuntimeError("The target window is minimized: WGC cannot reliably capture minimized clients")
    pid = wintypes.DWORD()
    user32.GetWindowThreadProcessId(wintypes.HWND(hwnd), ctypes.byref(pid))
    if expected_pid is not None and pid.value != expected_pid:
        raise RuntimeError(f"Wrong capture target: HWND {hwnd} owns PID {pid.value}, expected {expected_pid}")
    text = ctypes.create_unicode_buffer(300)
    user32.GetWindowTextW(wintypes.HWND(hwnd), text, len(text))
    if not text.value:
        raise RuntimeError("Target HWND has no identifying window title")
    return {"hwnd": hwnd, "pid": pid.value, "title": text.value, "backend": "Windows.Graphics.Capture"}


@dataclass
class Validation:
    verified: bool
    reason: str
    target: dict
    captured_frames: int
    encoded_frames: int
    source_width: int
    source_height: int
    output_width: int
    output_height: int
    duration_sec: float
    mean_luma: float
    spatial_std: float
    motion_score: float
    changed_sample_pairs: int
    sample_pairs: int
    reference_score: float | None
    reference_path: str | None


def aspect_frame(image: np.ndarray, width: int, height: int) -> np.ndarray:
    """Letterbox a BGRA image, never distort the scene to fit the requested file."""
    ih, iw = image.shape[:2]
    ratio = min(width / iw, height / ih)
    tw, th = max(2, int(iw * ratio) // 2 * 2), max(2, int(ih * ratio) // 2 * 2)
    resized = cv2.resize(image, (tw, th), interpolation=cv2.INTER_AREA if ratio < 1 else cv2.INTER_LINEAR)
    dest = np.zeros((height, width, 4), dtype=np.uint8)
    dest[:, :, 3] = 255
    dest[(height - th) // 2:(height + th) // 2, (width - tw) // 2:(width + tw) // 2] = resized
    return dest


def metrics(frames: list[np.ndarray], reference: Path | None) -> tuple:
    if not frames:
        return (0., 0., 0., 0, 0, None)
    gray = [cv2.cvtColor(f, cv2.COLOR_BGRA2GRAY) for f in frames]
    # Exclude black letterboxing; inspect centre where the game actor should be.
    centre = [im[im.shape[0] // 6:im.shape[0] * 5 // 6,
                 im.shape[1] // 6:im.shape[1] * 5 // 6] for im in gray]
    brightness = float(np.mean([np.mean(im) for im in centre]))
    std = float(np.mean([np.std(im) for im in centre]))
    pairs = [float(np.mean(cv2.absdiff(a, b) > 9)) for a, b in zip(centre, centre[1:])]
    motion = float(max(pairs, default=0.0))
    changed = sum(p >= 0.006 for p in pairs)
    ref_score = None
    if reference:
        original = cv2.imread(str(reference))
        if original is None:
            raise RuntimeError(f"Reference screenshot unreadable: {reference}")
        # Coarse HSV colour-distribution agreement tolerates camera pans and
        # animated subjects, but rejects completely unrelated desktop windows.
        hsv_ref = cv2.cvtColor(original, cv2.COLOR_BGR2HSV)
        ref_hist = cv2.calcHist([hsv_ref], [0, 1], None, [24, 20], [0, 180, 0, 256])
        cv2.normalize(ref_hist, ref_hist)
        scores = []
        for frame in frames:
            hsv = cv2.cvtColor(frame, cv2.COLOR_BGRA2BGR)
            hsv = cv2.cvtColor(hsv, cv2.COLOR_BGR2HSV)
            hist = cv2.calcHist([hsv], [0, 1], None, [24, 20], [0, 180, 0, 256])
            cv2.normalize(hist, hist)
            scores.append(cv2.compareHist(ref_hist, hist, cv2.HISTCMP_CORREL))
        ref_score = float(max(scores))
    return brightness, std, motion, changed, len(pairs), ref_score


def record_window(
    hwnd: int, output: Path, *, seconds: float = 10, fps: int = 20,
    width: int = 960, height: int = 540, expected_pid: int | None = None,
    reference: Path | None = None, min_reference_score: float = 0.22,
    stop_when=None, ffmpeg: str | None = None,
) -> Validation:
    """Record/validate one HWND; never returns a valid result for wrong/static video.

    A partial rejected recording remains in output for debugging, with a JSON
    verdict. Callers MUST check verified before publishing or attaching.
    """
    if not 1 <= seconds <= 120 or not 2 <= fps <= 60:
        raise ValueError("Seconds 1–120 and frame rate 2–60 required")
    if width < 320 or height < 240 or width % 2 or height % 2:
        raise ValueError("Even output width/height >=320x240 required")
    target = validate_hwnd(hwnd, expected_pid)
    ffmpeg = ffmpeg or shutil.which("ffmpeg")
    if not ffmpeg:
        raise RuntimeError("FFmpeg executable not found")
    output = Path(output).resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    validation_path = output.with_suffix(".validation.json")
    if reference:
        reference = Path(reference).resolve()
        if not reference.is_file():
            raise FileNotFoundError(reference)

    latest = None
    updates = 0
    received = threading.Event()
    closed = threading.Event()
    lock = threading.Lock()
    # WGC notifies on rendered changes. Resample at fixed fps to encode
    # honest wall-time, including pauses, without uneven interframe durations.
    capture = WindowsCapture(window_hwnd=hwnd, cursor_capture=False,
                             draw_border=False, minimum_update_interval=max(8, int(1000 / fps)))

    @capture.event
    def on_frame_arrived(frame, control):
        nonlocal latest, updates
        data = np.ascontiguousarray(frame.frame_buffer).copy()
        with lock:
            latest = data
            updates += 1
        received.set()

    @capture.event
    def on_closed():
        closed.set()

    controller = capture.start_free_threaded()
    proc = None
    validation = None
    try:
        if not received.wait(timeout=6):
            raise RuntimeError("No real GPU/window frames arrived within 6 seconds")
        with lock:
            first = latest.copy()
        h0, w0 = first.shape[:2]
        command = [ffmpeg, "-hide_banner", "-loglevel", "warning", "-y",
                   "-f", "rawvideo", "-pixel_format", "bgra",
                   "-video_size", f"{width}x{height}", "-framerate", str(fps),
                   "-i", "pipe:0", "-an", "-c:v", "libx264",
                   "-preset", "ultrafast", "-crf", "21", "-pix_fmt", "yuv420p",
                   str(output)]
        proc = subprocess.Popen(command, stdin=subprocess.PIPE,
                                stdout=subprocess.DEVNULL,
                                stderr=subprocess.PIPE, creationflags=CREATE_NO_WINDOW)
        interval = 1.0 / fps
        start = time.monotonic()
        recorded = 0
        sampled = []
        next_sample = 0.
        while recorded < math.ceil(seconds * fps):
            if closed.is_set():
                break
            if stop_when and stop_when():
                break
            wait_time = start + recorded * interval - time.monotonic()
            if wait_time > 0:
                time.sleep(wait_time)
            with lock:
                current = latest
                count = updates
            if current is None:
                continue
            if current.shape[:2] != (h0, w0):
                # Source resizing is detectable; do not silently warp crop.
                raise RuntimeError("Captured game resized during clip; review window placement")
            scaled = aspect_frame(current, width, height)
            try:
                proc.stdin.write(scaled.tobytes())
            except (OSError, BrokenPipeError) as error:
                raise RuntimeError(f"Video encoder stopped: {error}") from error
            elapsed = time.monotonic() - start
            if elapsed >= next_sample:
                # Bounded diagnostics ~2fps; never retain full uncompressed clip.
                sampled.append(cv2.resize(scaled, (192, 108), interpolation=cv2.INTER_AREA))
                next_sample += 0.55
            recorded += 1
        proc.stdin.close()
        proc.stdin = None
        stderr = proc.communicate(timeout=25)[1].decode(errors="replace")
        if proc.returncode:
            raise RuntimeError(f"FFmpeg exited {proc.returncode}: {stderr[-1400:]}")
        b, std, motion, changed, pairs, score = metrics(sampled, reference)
        if recorded < fps * min(seconds, 3.):
            reason = "Video too short to validate useful motion"
        elif b < 12 or std < 8:
            reason = "Blank/flat capture: target appears absent or not rendering"
        elif pairs < 2 or (changed < 2 and motion < 0.01):
            reason = "Static/repeated frames: movement not demonstrated"
        elif score is not None and score < min_reference_score:
            reason = f"Captured window differs from expected in-game reference ({score:.3f} < {min_reference_score})"
        else:
            reason = "Verified target HWND, nonblank scene, moving frames" + (
                ", reference scene similarity" if score is not None else "")
        validation = Validation(
            verified=reason.startswith("Verified"),
            reason=reason, target=target, captured_frames=count,
            encoded_frames=recorded, source_width=w0, source_height=h0,
            output_width=width, output_height=height,
            duration_sec=recorded / fps, mean_luma=round(b, 2),
            spatial_std=round(std, 2), motion_score=round(motion, 4),
            changed_sample_pairs=changed, sample_pairs=pairs,
            reference_score=round(score, 4) if score is not None else None,
            reference_path=str(reference) if reference else None,
        )
        validation_path.write_text(json.dumps(asdict(validation), indent=2), encoding="utf-8")
        if not validation.verified:
            raise RuntimeError(f"Rejected recording: {reason}; report={validation_path}")
        return validation
    finally:
        controller.stop()
        # Avoid leaving orphan FFmpeg encoding when the target disappears.
        if proc and proc.poll() is None:
            if proc.stdin and not proc.stdin.closed:
                proc.stdin.close()
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--hwnd", type=int, required=True)
    parser.add_argument("--expected-pid", type=int)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--reference", type=Path)
    parser.add_argument("--min-reference-score", type=float, default=0.22)
    parser.add_argument("--seconds", type=float, default=10)
    parser.add_argument("--fps", type=int, default=20)
    parser.add_argument("--width", type=int, default=960)
    parser.add_argument("--height", type=int, default=540)
    parser.add_argument("--ffmpeg", default=None)
    args = parser.parse_args()
    result = record_window(args.hwnd, args.output, seconds=args.seconds,
                           expected_pid=args.expected_pid, fps=args.fps,
                           width=args.width, height=args.height,
                           reference=args.reference, min_reference_score=args.min_reference_score,
                           ffmpeg=args.ffmpeg)
    print(json.dumps(asdict(result), indent=2))


if __name__ == "__main__":
    main()
