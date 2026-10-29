#!/usr/bin/env python3
"""TaskBoard release bookkeeping boundary for an EXISTING authorized build/restart.

No build, restart, human acceptance, or automatic enrollment occurs here.
An integrator explicitly supplies the members/files in a manifest. The same
run_id survives script retry/crash. Missing evidence fails closed.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import re
import uuid

REPO = Path(__file__).resolve().parents[1]
CLI = Path(os.environ.get("TASKBOARD_CLI", REPO / "packages/task-cli/target/debug/taskboard.exe"))


def sha(path: Path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def git(repo: Path, *args):
    out = subprocess.run(["git", "-C", str(repo), *args],
                         capture_output=True, text=True, check=True)
    return out.stdout.strip()


def command(root: Path, *args):
    if not CLI.is_file():
        raise RuntimeError(f"TaskBoard CLI missing: {CLI}; build the CLI before release operations")
    p = subprocess.run([str(CLI), "--root", str(root), "--json", *args],
                       capture_output=True, text=True)
    try:
        data = json.loads(p.stdout)
        if "data" in data and "success" in data:
            if not data["success"]:
                raise RuntimeError(f"TaskBoard rejected {args[0]}: {data}")
            return data["data"]
        if p.returncode:
            raise RuntimeError(f"TaskBoard command failed: {data}")
        return data
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"TaskBoard command failed [{p.returncode}]: {p.stdout} {p.stderr}") from exc


def checkout_fingerprint(repo: Path):
    head = git(repo, "rev-parse", "HEAD")
    # This fingerprint includes the exact staged/unstaged bytes plus paths of
    # untracked files; selected untracked task files require independent hashes.
    status = git(repo, "status", "--porcelain=v1", "--untracked-files=all")
    diff = subprocess.run(["git", "-C", str(repo), "diff", "HEAD", "--binary"],
                          check=True, capture_output=True).stdout
    digest = hashlib.sha256(head.encode() + status.encode() + diff).hexdigest()
    return {"head": head, "checkout_digest": digest, "dirty": bool(status)}


def check_sources(payload):
    inputs = payload.get("inputs") or []
    if not inputs:
        raise ValueError("manifest must explicitly enumerate integrated task source inputs")
    gathered = []
    for item in inputs:
        for attr in ("task", "repository", "repo_path", "source_commit", "file_hashes"):
            if not item.get(attr):
                raise ValueError(f"missing source evidence {attr} in {item!r}")
        repo = Path(item["repo_path"]).resolve(strict=True)
        state = checkout_fingerprint(repo)
        source = item["source_commit"]
        if source.startswith("working-tree:"):
            if source[13:] != state["checkout_digest"]:
                raise ValueError(f"{item['task']}: working-tree source hash no longer matches live inputs")
        else:
            if not (len(source) == 40 and all(c in "0123456789abcdef" for c in source.lower())):
                raise ValueError(f"{item['task']}: provide exact 40-character Git source SHA")
            if subprocess.run(["git", "-C", str(repo), "merge-base", "--is-ancestor", source, state["head"]]).returncode:
                raise ValueError(f"{item['task']}: source {source} not proven integrated into {state['head']}")
        for relative, expected in item["file_hashes"].items():
            candidate = (repo / relative).resolve(strict=True)
            if not candidate.is_relative_to(repo) or not candidate.is_file():
                raise ValueError(f"{item['task']}: invalid source path {relative}")
            actual = sha(candidate)
            if actual != expected:
                raise ValueError(f"{item['task']}: {relative} changed ({expected} -> {actual})")
        gathered.append({
            "task": item["task"], "repository": item["repository"],
            "source_commit": source, "file_hashes": item["file_hashes"],
            "repo_path": str(repo), **state,
            "integrated_ref": f"HEAD:{state['head']};WORKTREE:{state['checkout_digest']}"
        })
    return gathered


def receipt_path(manifest):
    return Path(manifest).with_suffix(".release-receipt.json")


def preflight(args):
    path = Path(args.manifest).resolve(strict=True)
    manifest = json.loads(path.read_text(encoding="utf-8-sig"))
    root = Path(manifest["board_root"]).resolve(strict=True)
    rel = manifest["release_id"]
    run_id = manifest["run_id"]
    if len(run_id) < 8:
        raise ValueError("run_id must be globally stable and >= 8 characters")
    sources = check_sources(manifest)
    # One Done task may have integrated changes in several repositories. Only
    # exact duplicate task/repository rows are ambiguous.
    if len({(x["task"], x["repository"]) for x in sources}) != len(sources):
        raise ValueError("duplicate task/repository source entries")
    rp = receipt_path(path)
    if rp.exists():
        prior = json.loads(rp.read_text(encoding="utf-8"))
        if (prior.get("run_id") != run_id or
                prior.get("source_manifest_sha256") != sha(path) or
                prior.get("release_id") != rel):
            raise ValueError("prior immutable preflight receipt conflicts with current manifest")
        if prior.get("result") != "prepared_only":
            raise ValueError(
                f"run_id {run_id} already recorded {prior.get('result')}; "
                "a subsequent build needs a NEW immutable manifest/run_id "
                "(use outcome only to reconcile an earlier UNKNOWN)"
            )
        # A repeated preflight is permitted only for the same prepared build.
        # This also rechecks checkout/file hashes via check_sources above.
        return prior
    existing = command(root, "release", "show", rel)
    existing_members = {m["task_id"] for m in existing["members"]}
    for item in sources:
        task = command(root, "task", "show", item["task"])["task"]
        if not task.get("completed") or task["id"] not in existing_members:
            raise ValueError(f"{item['task']}: not an already-enrolled Done task")
        command(root, "release", "source", rel, item["task"],
                "--repo", item["repository"], "--commit", item["source_commit"],
                "--integrated-ref", item["integrated_ref"],
                "--evidence", f"preflight {run_id}; file hashes verified; manifest {sha(path)}")
    # The core prepare operation snapshots ALL ready members. Refuse a partial
    # source manifest BEFORE freezing a mismatched attempt (not afterward).
    current = command(root, "release", "show", rel)
    ready = {m["task_id"] for m in current["members"] if m["stage"] == "ready"}
    declared = {x["task"] for x in sources}
    if ready != declared:
        raise ValueError(f"preflight would accidentally freeze ready members {ready - declared}; "
                         f"declared members not ready {declared - ready}")
    prepared = command(root, "release", "prepare", rel,
                       "--run-id", run_id, "--evidence",
                       f"preflight {run_id}; immutable source manifest={sha(path)}")
    matching = [x for x in prepared["attempts"] if x.get("run_id") == run_id]
    if len(matching) != 1:
        raise RuntimeError(f"run_id {run_id} did not yield exactly one immutable attempt")
    attempt = matching[0]
    claimed = {m["task_id"] for m in attempt["members"]}
    if claimed != {x["task"] for x in sources}:
        raise ValueError(f"attempt contains unexpected members {claimed}; abort build")
    record = {
        "board_root": str(root), "release_id": rel, "run_id": run_id,
        "attempt": attempt["number"], "source_manifest": str(path),
        "source_manifest_sha256": sha(path), "sources": sources,
        "preflight_at": time.time(), "result": "prepared_only"
    }
    rp = receipt_path(path)
    if rp.exists():
        previous = json.loads(rp.read_text(encoding="utf-8"))
        for key in ("run_id", "attempt", "source_manifest_sha256"):
            if previous[key] != record[key]:
                raise ValueError(f"existing receipt conflicts: {key}")
        return previous
    rp.write_text(json.dumps(record, indent=2), encoding="utf-8")
    return record


def outcome(args):
    path = Path(args.manifest).resolve(strict=True)
    rp = receipt_path(path)
    record = json.loads(rp.read_text(encoding="utf-8"))
    if record["source_manifest_sha256"] != sha(path):
        raise ValueError("source manifest changed after immutable preflight")
    prior_status = record.get("result", "prepared_only")
    if prior_status not in ("prepared_only", "unknown", args.status):
        raise ValueError(f"attempt already finalized as {prior_status}; use a new run_id")
    root = Path(record["board_root"])
    status = args.status
    parts = [f"run_id={record['run_id']}"]
    if args.log:
        logfile = Path(args.log).resolve(strict=True)
        parts.append(f"log={logfile} SHA256={sha(logfile)}")
    if status == "deployed":
        # The outcome API can also be used independently of T3's runner. Its
        # artifact/activation receipt must not bless a checkout that changed
        # after the immutable preflight without a fresh independently reviewed run.
        current_sources = check_sources(json.loads(path.read_text(encoding="utf-8-sig")))
        if current_sources != record["sources"]:
            raise ValueError("source inputs changed since preflight; deployed evidence cannot be attributed to this attempt")
        if not args.artifact or not args.activation_receipt:
            raise ValueError("deployed requires actual installed artifact AND independent activation receipt")
        artifact = Path(args.artifact).resolve(strict=True)
        activation = Path(args.activation_receipt).resolve(strict=True)
        if not artifact.is_file() or not activation.is_file():
            raise ValueError("artifact/activation receipt not a real file")
        parts += [f"artifact={artifact} SHA256={sha(artifact)}",
                  f"activation_receipt={activation} SHA256={sha(activation)}"]
    else:
        parts.append(f"operator_recorded_{status}; no deployment success inferred")
    if not args.log and status in ("build_failed", "unknown"):
        raise ValueError("failure/unknown requires an actual diagnostic log")
    evidence = "; ".join(parts)
    if prior_status == status:
        if record.get("outcome_evidence") != evidence:
            raise ValueError(f"attempt already recorded {status} with different evidence")
        return {"attempt": record["attempt"], "result": status, "record_status": status}
    data = command(root, "release", "outcome", record["release_id"],
                   str(record["attempt"]), status, "--run-id", record["run_id"],
                   "--evidence", evidence)
    record["result"] = status
    record["outcome_evidence"] = evidence
    rp.write_text(json.dumps(record, indent=2), encoding="utf-8")
    return {"attempt": record["attempt"], "result": status,
            "record_status": next(x["status"] for x in data["attempts"] if x["number"] == record["attempt"])}



def draft(args):
    """Build a REVIEWABLE source manifest from exact commits in Done task notes.

    This is deliberately NOT a release preflight. An integrator must inspect
    the resulting task/commit/file mapping before an authorized build uses it.
    Ambiguous, unmerged, missing-file or unidentified work is left out.
    """
    root = Path(args.root).resolve(strict=True)
    repo = Path(args.repo).resolve(strict=True)
    output = Path(args.output).resolve()
    if output.exists():
        raise ValueError(f"refusing to overwrite manifest/receipt lineage: {output}")
    release = command(root, "release", "show", args.release)
    inputs, skipped = [], []
    note_sha = re.compile(
        r"\b(?:commit(?:ted)?|revision|source\s+sha)\s*(?:[:=#]\s*)?([0-9a-f]{7,40})\b",
        re.IGNORECASE,
    )
    for member in release["members"]:
        tid = member["task_id"]
        if member["stage"] not in ("awaiting_integration", "ready"):
            continue
        task = command(root, "task", "show", tid)["task"]
        if task.get("column") != "done" or not task.get("completed"):
            skipped.append({"task": tid, "reason": "not Done"})
            continue
        notes = "\n".join(str(task.get(k) or "") for k in ("ai_notes", "notes"))
        candidates = set()
        for short_sha in note_sha.findall(notes):
            try:
                full = git(repo, "rev-parse", "--verify", f"{short_sha}^{{commit}}")
                if subprocess.run(
                    ["git", "-C", str(repo), "merge-base", "--is-ancestor", full, "HEAD"],
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                ).returncode == 0:
                    candidates.add(full)
            except subprocess.CalledProcessError:
                continue
        if len(candidates) != 1:
            skipped.append({"task": tid, "reason": f"{len(candidates)} valid unique source commits"})
            continue
        commit = next(iter(candidates))
        changed = git(repo, "diff-tree", "--root", "--no-commit-id", "--name-only", "-r", commit)
        paths = changed.splitlines()
        if not paths:
            skipped.append({"task": tid, "reason": "commit changes no files"})
            continue
        hashes = {}
        for relative in paths:
            candidate = (repo / relative).resolve()
            if not candidate.is_relative_to(repo) or not candidate.is_file():
                break
            hashes[relative] = sha(candidate)
        if len(hashes) != len(paths):
            skipped.append({"task": tid, "reason": "commit contains absent/deleted files"})
            continue
        inputs.append({
            "task": tid, "repository": args.repository,
            "repo_path": str(repo), "source_commit": commit,
            "file_hashes": hashes,
        })
    if not inputs:
        raise ValueError(f"no uniquely source-proven Done members; {len(skipped)} require integration review")
    payload = {
        "board_root": str(root), "release_id": args.release,
        "run_id": str(uuid.uuid4()), "inputs": inputs,
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("x", encoding="utf-8") as stream:
        json.dump(payload, stream, indent=2)
        stream.write("\n")
    return {
        "manifest": str(output), "run_id": payload["run_id"],
        "candidate_count": len(inputs), "skipped_count": len(skipped),
        "skipped": skipped,
        "notice": "DRAFT ONLY; review source mapping before using --taskboard-manifest in an authorized T3 restart",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    d = sub.add_parser("draft", help="Read-only source discovery; output explicit REVIEW-REQUIRED manifest")
    d.add_argument("--root", required=True, help="Canonical TaskBoard Tasks directory")
    d.add_argument("--release", required=True)
    d.add_argument("--repo", required=True, help="Exact source repository checkout")
    d.add_argument("--repository", required=True, help="Stable repository label")
    d.add_argument("--output", required=True, help="New manifest path; never overwrites")
    a = sub.add_parser("preflight", help="Source validation + immutable release attempt; NO app actions")
    a.add_argument("manifest")
    b = sub.add_parser("outcome", help="Record evidence AFTER existing authorized script finishes")
    b.add_argument("manifest")
    b.add_argument("--status", choices=("deployed", "build_failed", "unknown"), required=True)
    b.add_argument("--log")
    b.add_argument("--artifact")
    b.add_argument("--activation-receipt")
    args = parser.parse_args()
    try:
        result = (draft(args) if args.action == "draft" else
                  preflight(args) if args.action == "preflight" else outcome(args))
        print(json.dumps({"ok": True, "result": result}, indent=2))
    except (ValueError, RuntimeError, OSError, subprocess.CalledProcessError) as exc:
        print(json.dumps({"ok": False, "error": str(exc)}), file=sys.stderr)
        sys.exit(2)


if __name__ == "__main__":
    main()

