"""Agent-invokable Review Inbox clip recorder/encoder (Windows + FFmpeg).

Examples:
  py -B scripts/review_video.py start --state Saved/clip.json --title "WarTribes" --raw Saved/clip.mkv
  py -B scripts/review_video.py stop --state Saved/clip.json
  py -B scripts/review_video.py encode Saved/clip.mkv Saved/clip.mp4 --poster Saved/clip.webp
  py -B scripts/review_video.py record --title "WarTribes" --seconds 8 --raw Saved/clip.mkv
"""
from __future__ import annotations

import argparse
import ctypes
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

FALLBACK_FFMPEG = Path(r"B:\GDrive\Tools\06 Learning\subs2srs\Utils\ffmpeg\ffmpeg.exe")
FLAGS = 0x08000000 if os.name == "nt" else 0  # CREATE_NO_WINDOW
LIMIT = 180


def process_alive(pid: object) -> bool:
    """Check a recorder/worker PID without requiring an optional pip package."""
    if not isinstance(pid, int) or pid <= 0:
        return False
    if os.name == "nt":
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        open_process = kernel.OpenProcess
        open_process.argtypes = (ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong)
        open_process.restype = ctypes.c_void_p
        close = kernel.CloseHandle
        close.argtypes = (ctypes.c_void_p,)
        wait = kernel.WaitForSingleObject
        wait.argtypes = (ctypes.c_void_p, ctypes.c_ulong)
        wait.restype = ctypes.c_ulong
        handle = open_process(0x00100000, 0, pid)  # SYNCHRONIZE
        if not handle:
            return False
        try:
            return wait(handle, 0) == 0x00000102  # WAIT_TIMEOUT
        finally:
            close(handle)
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False


def ffmpeg():
    found = shutil.which("ffmpeg")
    if found:
        return found
    if FALLBACK_FFMPEG.is_file():
        return str(FALLBACK_FFMPEG)
    raise RuntimeError("FFmpeg unavailable. Add it to PATH (or install at the documented shared path).")


def command(args: list[str], **kwargs):
    return subprocess.run([ffmpeg(), "-hide_banner", "-loglevel", "error", "-y", *args],
                          check=True, creationflags=FLAGS, **kwargs)


def capture_args(args):
    if not 1 <= args.fps <= 60:
        raise ValueError("Capture FPS must be 1–60")
    target = f"title={args.title}" if args.title else "desktop"
    # GDI window capture works without focus on a visible window but can be
    # occluded, minimized or fail for GPU surfaces. Validate the actual frames.
    capture = ["-f", "gdigrab", "-framerate", str(args.fps), "-draw_mouse", "0",
               "-i", target]
    if args.audio_device:
        # Explicit opt-in only. dshow devices can include MICROPHONES, so
        # callers must choose the intended loopback/cable capture device.
        capture += ["-f", "dshow", "-i", "audio=" + args.audio_device]
    capture += ["-t", str(args.max_seconds), "-c:v", "libx264",
                "-preset", "ultrafast", "-pix_fmt", "yuv420p"]
    if args.audio_device:
        capture += ["-map", "0:v:0", "-map", "1:a:0", "-c:a", "aac", "-b:a", "96k"]
    else:
        capture += ["-an"]
    return capture + ["-f", "matroska", str(args.raw.resolve())]


def save_state(path: Path, info: dict):
    temp = path.with_suffix(path.suffix + ".tmp")
    temp.write_text(json.dumps(info, indent=2), encoding="utf-8")
    os.replace(temp, path)


def worker(args):
    state = args.state.resolve()
    raw = args.raw.resolve()
    raw.parent.mkdir(parents=True, exist_ok=True)
    end = state.with_suffix(".stop")
    end.unlink(missing_ok=True)
    metadata = {"state": "starting", "pid": os.getpid(), "raw": str(raw),
                "target": args.title or "desktop", "fps": args.fps,
                "audioDevice": args.audio_device or None, "startedAt": time.time()}
    save_state(state, metadata)
    with state.with_suffix(".log").open("ab") as log:
        proc = subprocess.Popen([ffmpeg(), "-hide_banner", "-loglevel", "warning", "-y",
                                 *capture_args(args)], stdin=subprocess.PIPE,
                                stdout=subprocess.DEVNULL, stderr=log,
                                creationflags=FLAGS)
        metadata.update(state="recording", recorderPid=proc.pid)
        save_state(state, metadata)
        deadline = time.monotonic() + args.max_seconds + 10
        while proc.poll() is None and time.monotonic() < deadline:
            if end.exists():
                try:
                    proc.stdin.write(b"q\n")
                    proc.stdin.flush()
                except OSError:
                    pass
                break
            time.sleep(0.1)
        try:
            code = proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
            code = proc.wait()
        metadata.update(state="done" if code == 0 and raw.is_file() and raw.stat().st_size > 0 else "failed",
                        code=code, bytes=raw.stat().st_size if raw.is_file() else 0,
                        endedAt=time.time())
        save_state(state, metadata)
        end.unlink(missing_ok=True)


def start(args):
    args.state = args.state.resolve()
    args.state.parent.mkdir(parents=True, exist_ok=True)
    if args.state.is_file():
        old = json.loads(args.state.read_text(encoding="utf-8"))
        if old.get("state") in ("starting", "recording"):
            if process_alive(old.get("pid")):
                raise RuntimeError("Recorder worker is still active; stop it first")
            if process_alive(old.get("recorderPid")):
                raise RuntimeError("Worker crashed but FFmpeg is still recording; wait for its bounded timeout, then retry")
            # A crashed worker cannot leave this state locked forever. Preserve
            # its state and possibly salvageable intermediate instead of
            # silently overwriting the user's last captured evidence.
            stamp = int(time.time())
            previous = args.state.with_name(args.state.name + f".recovered-{stamp}.json")
            args.state.replace(previous)
            old_raw = Path(old.get("raw", ""))
            if old_raw.is_file() and old_raw.stat().st_size:
                recovered = old_raw.with_name(old_raw.stem + f".recovered-{stamp}" + old_raw.suffix)
                old_raw.replace(recovered)
                print(f"Recovered stale recorder state: {previous}; preserved raw: {recovered}", file=sys.stderr)
            else:
                print(f"Recovered stale recorder state: {previous}", file=sys.stderr)
        else:
            # Do not let start() mistake the last run's "done"/"failed"
            # state for the new worker's status before it gets CPU time.
            previous = args.state.with_name(args.state.name + f".previous-{time.time_ns()}.json")
            args.state.replace(previous)
    args.state.with_suffix(".stop").unlink(missing_ok=True)
    cmd = [sys.executable, "-B", str(Path(__file__).resolve()), "_worker",
           "--state", str(args.state), "--raw", str(args.raw.resolve()),
           "--fps", str(args.fps), "--max-seconds", str(args.max_seconds)]
    if args.title:
        cmd.extend(["--title", args.title])
    if args.audio_device:
        cmd.extend(["--audio-device", args.audio_device])
    subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                     stderr=subprocess.DEVNULL, creationflags=FLAGS,
                     close_fds=True)
    for _ in range(45):
        if args.state.is_file():
            status = json.loads(args.state.read_text(encoding="utf-8"))
            if status["state"] in ("recording", "done", "failed"):
                print(json.dumps(status))
                if status["state"] == "failed":
                    raise RuntimeError("Recorder exited before start; check " + str(args.state.with_suffix(".log")))
                return
        time.sleep(0.1)
    raise RuntimeError("Recorder did not start; check FFmpeg and state log")


def stop(args):
    state = args.state.resolve()
    if not state.is_file():
        raise FileNotFoundError(f"No recording state: {state}")
    info = json.loads(state.read_text(encoding="utf-8"))
    if info["state"] not in ("recording", "starting"):
        print(json.dumps(info))
        return
    state.with_suffix(".stop").write_text("stop", encoding="ascii")
    for _ in range(120):
        current = json.loads(state.read_text(encoding="utf-8"))
        if current["state"] in ("starting", "recording") and not process_alive(current.get("pid")):
            raise RuntimeError("Recorder worker crashed before finalizing. The raw Matroska may still be recoverable; rerun 'start' after the FFmpeg process has exited.")
        if current["state"] in ("done", "failed"):
            print(json.dumps(current))
            if current["state"] == "failed":
                raise RuntimeError("FFmpeg failed; inspect " + str(state.with_suffix(".log")))
            return
        time.sleep(0.1)
    raise RuntimeError("Recorder did not stop cleanly; check state log")


def encode(args):
    source = args.source.resolve(strict=True)
    output = args.output.resolve()
    if source == output:
        raise ValueError("Output must differ from input")
    output.parent.mkdir(parents=True, exist_ok=True)
    vf = f"fps={args.fps},scale='min({args.width},iw)':-2:flags=lanczos"
    # H.264 + yuv420p + faststart is widely decodable on Android/iOS/browser.
    # No upscaling, constrained bitrate/CRF; audio is normalized only if present.
    command(["-i", str(source), "-vf", vf, "-c:v", "libx264", "-preset", "veryfast",
             "-crf", str(args.crf), "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "96k",
             "-movflags", "+faststart", str(output)])
    if args.poster:
        poster(output, args.poster)
    print(json.dumps({"mp4": str(output), "bytes": output.stat().st_size,
                      "poster": str(args.poster) if args.poster else None}))
    if output.stat().st_size > 25 * 1024 * 1024:
        print("WARNING: Clip exceeds TaskBoard's 25 MiB synced evidence threshold; trim/reencode.",
              file=sys.stderr)


def poster(source: Path, output: Path):
    output = output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    command(["-ss", "0.4", "-i", str(source.resolve()), "-frames:v", "1",
             "-vf", "scale='min(640,iw)':-2", str(output)])


def trim(args):
    source = args.source.resolve(strict=True)
    output = args.output.resolve()
    if source == output:
        raise ValueError("Output must differ from input")
    output.parent.mkdir(parents=True, exist_ok=True)
    command(["-ss", str(args.start), "-i", str(source), "-t", str(args.seconds),
             "-c:v", "libx264", "-preset", "veryfast", "-crf", "27",
             "-pix_fmt", "yuv420p", "-c:a", "aac", "-movflags", "+faststart", str(output)])
    print(str(output))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    for name in ("start", "_worker", "record"):
        p = sub.add_parser(name)
        p.add_argument("--raw", type=Path, required=True, help="Raw capture filename (.mkv)")
        p.add_argument("--title", help="Exact visible window title; omitted = entire desktop")
        p.add_argument("--audio-device", default="", help="Explicit opt-in DirectShow audio device, e.g. VB-Audio cable output; default silent")
        p.add_argument("--fps", type=int, default=20)
        p.add_argument("--max-seconds", type=int, default=LIMIT)
        if name != "record":
            p.add_argument("--state", type=Path, required=True)
        else:
            p.add_argument("--seconds", type=float, required=True)
    p = sub.add_parser("stop")
    p.add_argument("--state", type=Path, required=True)
    p = sub.add_parser("encode")
    p.add_argument("source", type=Path)
    p.add_argument("output", type=Path)
    p.add_argument("--poster", type=Path)
    p.add_argument("--fps", type=int, default=20)
    p.add_argument("--width", type=int, default=1280)
    p.add_argument("--crf", type=int, default=27)
    p = sub.add_parser("poster")
    p.add_argument("source", type=Path)
    p.add_argument("output", type=Path)
    p = sub.add_parser("trim")
    p.add_argument("source", type=Path)
    p.add_argument("output", type=Path)
    p.add_argument("--start", type=float, default=0)
    p.add_argument("--seconds", type=float, required=True)
    args = parser.parse_args()
    if hasattr(args, "max_seconds") and not 1 <= args.max_seconds <= 180:
        parser.error("--max-seconds must be 1..180")
    if args.action == "start":
        start(args)
    elif args.action == "_worker":
        worker(args)
    elif args.action == "stop":
        stop(args)
    elif args.action == "record":
        args.max_seconds = max(1, min(180, round(args.seconds)))
        args.raw.parent.mkdir(parents=True, exist_ok=True)
        command(capture_args(args))
        print(str(args.raw.resolve()))
    elif args.action == "encode":
        encode(args)
    elif args.action == "poster":
        poster(args.source, args.output)
    elif args.action == "trim":
        trim(args)


if __name__ == "__main__":
    main()
