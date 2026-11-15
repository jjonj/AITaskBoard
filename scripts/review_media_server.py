"""TaskBoard Review Inbox media: board-scoped HTTP byte-range playback.

The same loopback server is used by TaskBoard desktop and the authenticated
OmniHub Gateway remote-tool proxy. Never accept filesystem paths from URLs.
"""
from __future__ import annotations

import argparse
import html
import json
import os
from pathlib import Path
import re
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import quote, unquote, urlsplit

VAULT = Path(os.environ.get(
    "TASKBOARD_REVIEW_VAULT",
    r"B:\GDrive\SharedWithPhone\Obsidian\jjj",
))
KINDS = {
    ".mp4": "video/mp4", ".webm": "video/webm", ".png": "image/png",
    ".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".webp": "image/webp",
    ".gif": "image/gif",
}
NAME = re.compile(r"^[A-Za-z0-9_-]{1,90}$")
TASK = re.compile(r"^[A-Za-z][A-Za-z0-9]*-[0-9]{4,}$")
ID = re.compile(r"^[A-Z]$")
MAX_MEDIA = 128 * 1024 * 1024


def source(board: str, task: str, round_num: int, artifact_id: str, poster: bool = False):
    if not NAME.fullmatch(board) or not TASK.fullmatch(task) or not 1 <= round_num <= 999 or not ID.fullmatch(artifact_id):
        raise ValueError("Invalid review media identifier")
    root = VAULT / board / "Tasks"
    manifest_path = root / "Reviews" / task / "manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8-sig"))
    round_data = next((r for r in manifest["rounds"] if r["number"] == round_num), None)
    if round_data is None:
        raise FileNotFoundError("Review round no longer exists")
    entry = next((a for a in round_data["artifacts"] if a["id"] == artifact_id), None)
    if entry is None:
        raise FileNotFoundError("Review artifact was removed")
    if entry.get("pruned"):
        raise FileNotFoundError("Review media was pruned")
    raw = entry.get("poster") if poster else entry["path"]
    if not raw:
        raise FileNotFoundError("Review poster unavailable")
    path = Path(raw)
    if not path.is_absolute():
        path = root / path
    real = path.resolve(strict=True)
    reviews = (root / "Reviews").resolve(strict=True)
    local = Path(os.environ.get("LOCALAPPDATA", "")) / "TaskBoard" / "review-media"
    # The manifest controls filenames but only this exact task/round may be
    # served, not a sibling task or unrelated file in the global local cache.
    own_review = (reviews / task).resolve(strict=True)
    own_round = (own_review / f"r{round_num}").resolve(strict=True)
    in_review = real.is_relative_to(own_round)
    in_local = False
    if local.is_dir():
        cache = local.resolve()
        if real.is_relative_to(cache):
            relative = real.relative_to(cache)
            in_local = (
                len(relative.parts) >= 4 and
                relative.parts[1] == task and
                relative.parts[2] == f"r{round_num}"
            )
    if not in_review and not in_local:
        raise ValueError("Review media path is outside the owning task and round")
    kind = KINDS.get(real.suffix.lower())
    if not kind or not real.is_file() or real.stat().st_size > MAX_MEDIA:
        raise ValueError("Unsupported or oversized review media")
    if poster and not kind.startswith("image/"):
        raise ValueError("Poster must be an image")
    if not poster and entry.get("kind") not in ("image", "video"):
        raise ValueError("Only native media is streamable")
    return real, kind, entry


def address(group: str, board: str, task: str, round_num: int, artifact_id: str):
    return f"../../{group}/{quote(board)}/{quote(task)}/{round_num}/{quote(artifact_id)}"


class Handler(BaseHTTPRequestHandler):
    server_version = "TaskBoardReviewMedia/1"

    def log_message(self, format: str, *args) -> None:
        # pythonw.exe (Task Scheduler hidden logon service) has no stderr:
        # BaseHTTPRequestHandler's default logging would otherwise raise and
        # close every HTTP connection before sending its response.
        pass

    def _send(self, code: int, message: bytes, content_type="text/plain; charset=utf-8"):
        self.send_response(code)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(message)))
        self.send_header("Cache-Control", "private, no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(message)

    def do_HEAD(self):
        self.do_GET()

    def do_GET(self):
        parts = [unquote(part) for part in urlsplit(self.path).path.strip("/").split("/")]
        if parts == ["health"]:
            return self._send(200, b"taskboard-review-media-ok")
        if parts == [""]:
            return self._send(200, b"""<!doctype html><html><meta name="viewport" content="width=device-width,initial-scale=1">
<title>TaskBoard Review Media</title><style>body{font:16px system-ui;background:#101820;color:#e2edf5;max-width:650px;margin:8vh auto;padding:20px}</style>
<h1>TaskBoard Review Media</h1><p>This service streams Review Inbox clips to desktop and authenticated OmniHub mobile browsers. Open an exact /review/BOARD/TASK link from TaskBoard.</p></html>""", "text/html; charset=utf-8")
        if len(parts) == 2 and parts[0] == "dashboard-media":
            filename = parts[1]
            if not re.fullmatch(r"[0-9a-f]{64}\.(?:mp4|webm|webp|png|jpe?g|gif)", filename):
                return self._send(404, b"Invalid Dashboard capture")
            folder = Path(r"D:\Wartribes\Saved\DevelopmentDashboard")
            try:
                index = json.loads((folder / "index.json").read_text(encoding="utf-8-sig"))
                if not any(entry.get("filename") == filename and not entry.get("hidden")
                           for entry in index.get("items", [])):
                    return self._send(404, b"Dashboard capture not published")
                media = (folder / "media" / filename).resolve(strict=True)
                if media.stat().st_size > MAX_MEDIA:
                    return self._send(404, b"Dashboard capture exceeds 128 MiB")
                return self._stream(media, KINDS[media.suffix.lower()])
            except (OSError, ValueError, KeyError, json.JSONDecodeError):
                return self._send(404, b"Dashboard capture unavailable")
        if len(parts) == 5 and parts[0] in ("media", "poster"):
            _, board, task, number, item = parts
            try:
                if not number.isdecimal():
                    raise ValueError("Invalid review round")
                path, mime, _ = source(board, task, int(number), item, parts[0] == "poster")
                return self._stream(path, mime)
            except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
                return self._send(404, str(error).encode("utf-8", "replace"))
        if len(parts) == 3 and parts[0] == "review":
            return self._review(parts[1], parts[2])
        return self._send(404, b"Unknown review endpoint")

    def _stream(self, path: Path, mime: str):
        size = path.stat().st_size
        if size <= 0:
            return self._send(404, b"Review media is empty")
        start, end = 0, size - 1
        requested = self.headers.get("Range", "")
        if requested:
            match = re.fullmatch(r"bytes=(\d*)-(\d*)", requested.strip())
            if not match or (not match[1] and not match[2]):
                return self._invalid_range(size)
            if not match[1]:
                length = int(match[2])
                start = max(0, size - length)
            else:
                start = int(match[1])
                if match[2]:
                    end = min(end, int(match[2]))
            if start >= size or start > end:
                return self._invalid_range(size)
        count = end - start + 1
        self.send_response(206 if requested else 200)
        self.send_header("Content-Type", mime)
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Length", str(count))
        if requested:
            self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
        self.send_header("Cache-Control", "private, max-age=600")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.end_headers()
        if self.command == "HEAD":
            return
        try:
            with path.open("rb") as media:
                media.seek(start)
                while count > 0:
                    chunk = media.read(min(count, 256 * 1024))
                    if not chunk:
                        break
                    self.wfile.write(chunk)
                    count -= len(chunk)
        except (ConnectionResetError, BrokenPipeError):
            pass  # User stopped watching/seeking; no failed review.

    def _invalid_range(self, size: int):
        self.send_response(416)
        self.send_header("Content-Range", f"bytes */{size}")
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Length", "0")
        self.end_headers()

    def _review(self, board: str, task: str):
        if not NAME.fullmatch(board) or not TASK.fullmatch(task):
            return self._send(404, b"Invalid board/task")
        manifest_path = VAULT / board / "Tasks" / "Reviews" / task / "manifest.json"
        try:
            manifest = json.loads(manifest_path.read_text(encoding="utf-8-sig"))
            r = manifest["rounds"][-1]
        except (OSError, IndexError, KeyError, ValueError, json.JSONDecodeError):
            return self._send(404, b"No review for this task")
        no = r["number"]
        panels = []
        for a in r["artifacts"]:
            identifier = a["id"]
            label = html.escape(a.get("label", identifier))
            if a.get("kind") not in ("image", "video"):
                panels.append(f"<section><h2>{label}</h2><p>Interactive review: open in desktop TaskBoard.</p></section>")
                continue
            try:
                source(board, task, no, identifier)
                media = html.escape(address("media", board, task, no, identifier))
                try:
                    source(board, task, no, identifier, True)
                    poster = html.escape(address("poster", board, task, no, identifier))
                except (OSError, ValueError, KeyError):
                    poster = ""
                if a["kind"] == "video":
                    view = (f'<video controls playsinline preload="metadata" '
                            f'poster="{poster}" src="{media}"></video>')
                else:
                    view = f'<img loading="lazy" src="{media}" alt="{label}">'
                panels.append(f'<section><h2>{label}</h2>{view}'
                              '<p class="error" hidden>Media could not be decoded or loaded.</p></section>')
            except (OSError, ValueError, KeyError):
                panels.append(f"<section><h2>{label}</h2><p>Media missing or unsupported.</p></section>")
        if not panels:
            panels = ["<p>This review has no native media.</p>"]
        judge = html.escape(r.get("judge", ""))
        body = f"""<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>{html.escape(task)} · Review evidence</title>
<style>html{{color-scheme:dark}}body{{margin:0;padding:15px;background:#101820;
color:#e2edf5;font:15px/1.5 system-ui,sans-serif}}header{{margin:auto;max-width:1024px}}
h1{{font-size:21px;margin:.3em 0}}h2{{font-size:15px;margin:0 0 10px}}
p{{color:#b6c8d5}}main{{display:grid;gap:14px;max-width:1024px;margin:20px auto}}
section{{border:1px solid #385267;padding:12px;border-radius:9px;background:#172331}}
video,img{{display:block;max-width:100%;max-height:80vh;margin:auto;object-fit:contain}}
video{{width:100%;background:#050a11}}.error{{color:#ffb8b8}}</style></head>
<body><header><small>{html.escape(board)} / TaskBoard Review · Round {no}</small>
<h1>{html.escape(task)}</h1><p>{judge}</p>
<p>Playback only. Submit verdicts and annotations in TaskBoard Inbox.</p></header>
<main>{"".join(panels)}</main><script>
document.querySelectorAll('video,img').forEach(el=>el.addEventListener('error',()=>{{
const err=el.parentElement.querySelector('.error');if(err)err.hidden=false;
}}));</script></body></html>"""
        self._send(200, body.encode("utf-8"), "text/html; charset=utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8785)
    args = parser.parse_args()
    server = ThreadingHTTPServer((args.host, args.port), Handler)
    print(f"TaskBoard Review Media listening on {args.host}:{args.port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
