"""Verified Win32 HWND video capture using Windows Graphics Capture (WGC).

Unlike FFmpeg gdigrab desktop/monitor or Ascent's DXGI monitor recordings,
WGC targets a specific native HWND, including when another window overlaps it.
Windows-only; requires "pip install --no-deps windows-capture" plus cv2/numpy.
No game/window focus, desktop input, microphone or hidden fallback to full-screen
recording. A clip fails validation rather than publishing unrelated windows.

CLI:
  py -B scripts/review_window_capture.py --hwnd 123456 --expected-pid 789 \
      --seconds 8 --output video.mp4 [--reference game_screenshot.png]
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
    crop: list[int]
    codec: str
    crf: int


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



def client_crop(hwnd: int, first: np.ndarray) -> tuple[int, int, int, int]:
    """Crop native client pixels, including PIE previews with extended title bars."""
    user32 = ctypes.windll.user32
    rect = wintypes.RECT()
    point = wintypes.POINT(0, 0)
    if not user32.GetClientRect(hwnd, ctypes.byref(rect)) or not user32.ClientToScreen(hwnd, ctypes.byref(point)):
        raise RuntimeError("Cannot locate captured window client region")
    bounds = wintypes.RECT()
    hr = ctypes.windll.dwmapi.DwmGetWindowAttribute(hwnd, 9, ctypes.byref(bounds), ctypes.sizeof(bounds))
    if hr != 0 and not user32.GetWindowRect(hwnd, ctypes.byref(bounds)):
        raise RuntimeError("Cannot locate captured window frame")
    sh, sw = first.shape[:2]
    bw, bh = bounds.right - bounds.left, bounds.bottom - bounds.top
    if bw < 1 or bh < 1:
        raise RuntimeError("Invalid target window bounds")
    sx, sy = sw / bw, sh / bh
    x = max(0, round((point.x - bounds.left) * sx))
    y = max(0, round((point.y - bounds.top) * sy))
    w = min(sw - x, round((rect.right - rect.left) * sx))
    h = min(sh - y, round((rect.bottom - rect.top) * sy))
    # UE Preview can report its custom caption as client pixels. Inspect
    # the first real WGC frame rather than assuming a constant titlebar size.
    if y < 3 and h >= sh - 3 and sh >= 90:
        row_luma = first[:min(90, sh), :, :3].mean(axis=(1, 2))
        dark = float(np.median(row_luma[4:15]))
        if dark < 50:
            for top in range(20, min(80, len(row_luma) - 3)):
                if float(np.median(row_luma[top:top + 3])) > dark + 35:
                    x, y, w, h = 2, top, sw - 4, sh - top - 2
                    break
    if w < 320 or h < 240 or y > 120:
        raise RuntimeError(f"Invalid window client crop {(x, y, w, h)} from {sw}x{sh}")
    return x, y, w, h


def record_window(
    hwnd: int, output: Path, *, seconds: float = 10, fps: int = 30,
    width: int | None = None, height: int | None = None,
    expected_pid: int | None = None,
    reference: Path | None = None, min_reference_score: float = 0.22,
    stop_when=None, ffmpeg: str | None = None, crf: int = 12,
    crop: tuple[int, int, int, int] | None = None,
    include_window_chrome: bool = False, on_ready=None,
    require_motion: bool = True, min_update_ratio: float = 0.40,
) -> Validation:
    """Canonical review recorder: real WGC GPU frames directly to one sharp H.264 MP4.

    Full native client resolution (up to 1920 wide) and CRF 12 are the
    defaults for WAR animation, WDD gameplay videos and TaskBoard Inbox.
    No lossy intermediate/transcoding. The caller owns any Unreal settings
    overrides (PIE needs unthrottling/full-resolution before capture).
    """
    if not 1 <= seconds <= 120 or not 2 <= fps <= 60 or not 0 <= crf <= 25:
        raise ValueError("Seconds 1-120, frame rate 2-60, CRF 0-25 required")
    if (width is None) != (height is None):
        raise ValueError("Supply both output dimensions or neither (native resolution)")
    if width is not None and (width < 320 or height < 240 or width % 2 or height % 2):
        raise ValueError("Even output width/height >=320x240 required")
    if not 0 <= min_update_ratio <= 1:
        raise ValueError("min_update_ratio must be between 0 and 1")
    target = validate_hwnd(hwnd, expected_pid)
    if not ffmpeg:
        ffmpeg = shutil.which("ffmpeg")
        if not ffmpeg:
            try:
                import imageio_ffmpeg
                ffmpeg = imageio_ffmpeg.get_ffmpeg_exe()
            except (ImportError, OSError):
                fallback = Path(r"B:\GDrive\Tools\06 Learning\subs2srs\Utils\ffmpeg\ffmpeg.exe")
                ffmpeg = str(fallback) if fallback.is_file() else None
    if not ffmpeg:
        raise RuntimeError("FFmpeg executable not found")
    output = Path(output).resolve()
    if output.suffix.lower() not in (".mp4", ".mkv"):
        raise ValueError("Use a browser-playable MP4 (or MKV intermediate)")
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
    capture = WindowsCapture(window_hwnd=hwnd, cursor_capture=False,
                             draw_border=False, minimum_update_interval=max(1, round(1000 / fps)))

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
    try:
        if not received.wait(timeout=6):
            raise RuntimeError("No real GPU/window frames arrived within 6 seconds")
        with lock:
            first = latest.copy()
            first_updates = updates
        sh, sw = first.shape[:2]
        if first.ndim != 3 or first.shape[2] != 4:
            raise RuntimeError("WGC supplied unexpected non-BGRA pixels")
        if crop is not None:
            x, y, cw, ch = map(int, crop)
        elif include_window_chrome:
            x, y, cw, ch = 0, 0, sw, sh
        else:
            x, y, cw, ch = client_crop(hwnd, first)
        cw, ch = cw // 2 * 2, ch // 2 * 2
        if x < 0 or y < 0 or cw < 320 or ch < 240 or x + cw > sw or y + ch > sh:
            raise RuntimeError(f"Capture crop outside GPU frame: {(x, y, cw, ch)} from {sw}x{sh}")
        raw_w, raw_h = (width, height) if width is not None else (cw, ch)
        out_w = min(1920, raw_w)
        out_h = max(2, round(raw_h * out_w / raw_w / 2) * 2)
        # One encode directly from uncompressed BGRA. For native dimensions,
        # scale only if >1920 wide, using Lanczos. For explicit dimensions,
        # letterbox first for backward compatibility with existing callers.
        command = [ffmpeg, "-hide_banner", "-loglevel", "error", "-y",
                   "-f", "rawvideo", "-pix_fmt", "bgra",
                   "-s", f"{raw_w}x{raw_h}", "-r", str(fps),
                   "-i", "pipe:0", "-an", "-vf",
                   "scale='min(1920,iw)':-2:flags=lanczos",
                   "-c:v", "libx264", "-preset", "veryfast", "-crf", str(crf),
                   "-pix_fmt", "yuv420p"]
        if output.suffix.lower() == ".mp4":
            command += ["-movflags", "+faststart"]
        command.append(str(output))
        proc = subprocess.Popen(command, stdin=subprocess.PIPE,
                                stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                                creationflags=CREATE_NO_WINDOW, bufsize=0)
        if on_ready is not None:
            on_ready()  # Begin real animation playback only after capture is armed.
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
            if current.shape[:2] != (sh, sw):
                raise RuntimeError("Captured game resized during clip; do not stretch footage")
            frame = np.ascontiguousarray(current[y:y + ch, x:x + cw])
            if width is not None:
                frame = aspect_frame(frame, width, height)
            try:
                proc.stdin.write(frame.tobytes())
            except (OSError, BrokenPipeError) as error:
                raise RuntimeError(f"Video encoder stopped: {error}") from error
            elapsed = time.monotonic() - start
            if elapsed >= next_sample:
                sampled.append(cv2.resize(frame, (192, 108), interpolation=cv2.INTER_AREA))
                next_sample += 0.55
            recorded += 1
        proc.stdin.close()
        proc.stdin = None
        stderr = proc.communicate(timeout=90)[1].decode(errors="replace")
        if proc.returncode:
            raise RuntimeError(f"FFmpeg exited {proc.returncode}: {stderr[-1400:]}")
        b, std, motion, changed, pairs, score = metrics(sampled, reference)
        actual_updates = count - first_updates
        if recorded < fps * min(seconds, 3.):
            reason = "Video too short to validate"
        elif actual_updates < recorded * min_update_ratio:
            reason = f"Too few real WGC updates ({actual_updates}/{recorded}); background throttling/stalled game"
        elif b < 12 or std < 8:
            reason = "Blank/flat capture: target absent or not rendering"
        elif require_motion and (pairs < 2 or (changed < 2 and motion < 0.01)):
            reason = "Static/repeated frames: requested movement not demonstrated"
        elif score is not None and score < min_reference_score:
            reason = f"Captured window differs from expected in-game reference ({score:.3f} < {min_reference_score})"
        else:
            reason = "Verified target HWND, real GPU updates, sharp single-encode capture" + (
                ", reference scene similarity" if score is not None else "")
        validation = Validation(
            verified=reason.startswith("Verified"),
            reason=reason, target=target, captured_frames=actual_updates,
            encoded_frames=recorded, source_width=sw, source_height=sh,
            output_width=out_w, output_height=out_h,
            duration_sec=recorded / fps, mean_luma=round(b, 2),
            spatial_std=round(std, 2), motion_score=round(motion, 4),
            changed_sample_pairs=changed, sample_pairs=pairs,
            reference_score=round(score, 4) if score is not None else None,
            reference_path=str(reference) if reference else None,
            crop=[x, y, cw, ch], codec="h264", crf=crf,
        )
        validation_path.write_text(json.dumps(asdict(validation), indent=2), encoding="utf-8")
        if not validation.verified:
            raise RuntimeError(f"Rejected recording: {reason}; report={validation_path}")
        return validation
    finally:
        controller.stop()
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
    parser.add_argument("--fps", type=int, default=30)
    parser.add_argument("--width", type=int, default=None)
    parser.add_argument("--height", type=int, default=None)
    parser.add_argument("--crf", type=int, default=12)
    parser.add_argument("--ffmpeg", default=None)
    args = parser.parse_args()
    result = record_window(args.hwnd, args.output, seconds=args.seconds,
                           expected_pid=args.expected_pid, fps=args.fps,
                           width=args.width, height=args.height,
                           reference=args.reference, min_reference_score=args.min_reference_score,
                           ffmpeg=args.ffmpeg, crf=args.crf)
    print(json.dumps(asdict(result), indent=2))


if __name__ == "__main__":
    main()
