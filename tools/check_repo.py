"""Check the staged Git snapshot before publishing.

The checker deliberately reads Git's index and blob objects. Workspace-only
files cannot hide a staged path, credential, malformed JSON, or invalid link.
It checks repository boundaries; it does not claim that the application builds.
"""

from pathlib import Path, PurePosixPath
import hashlib
import json
import posixpath
import re
import subprocess
import sys
import tomllib
from urllib.parse import unquote, urlsplit


MAX_BYTES = 10 * 1024 * 1024
FORBIDDEN_FILE_NAMES = {"agents.md", "claude.md"}
FORBIDDEN_DIRECTORY_NAMES = {".agents", ".claude", ".local", "output", "_temp"}
LOCAL_PATHS = {
    "docs/acceptance.md", "docs/handoff.md", "docs/status.md",
    "docs/telegram.md", "docs/ux-audit.md",
    ".github/workflows/ci.yml", ".github/workflows/telegram-bot.yml",
    ".github/workflows/telegram-notify.yml",
    ".github/scripts/telegram-bot.mjs", ".github/scripts/tg-notify.mjs",
}
EXCEPTIONS = "references/repository-exceptions.json"
CREDENTIAL = re.compile(
    r"(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{50,}"
    r"|AKIA[0-9A-Z]{16}|sk-(?:proj-)?[A-Za-z0-9_-]{32,}"
    r"|-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----)"
)
LINK = re.compile(r"!?\[[^\]\n]*\]\(([^)\n]+)\)")
RESERVED = re.compile(r"^(?:con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)", re.I)
MACHINE_PATH = re.compile(
    r"(?:[A-Z]:[\\/](?:Users[\\/][^\\/\s\"']+|_[^\\/\s\"']+)"
    r"|/(?:home|Users)/(?!(?:user|username|example)(?:/|$))[^/\s\"']+)",
    re.I,
)


def git(*args):
    """Use Git's index and blob APIs instead of the working tree."""
    result = subprocess.run(["git", *args], capture_output=True)
    if result.returncode:
        raise RuntimeError("Git command failed: " + " ".join(args[:2]))
    return result.stdout


def ignored(path, tracked_paths):
    """Evaluate project rules without inheriting a developer's global excludes."""
    result = subprocess.run(
        [
            "git",
            "-c",
            "core.excludesFile=",
            "check-ignore",
            "--no-index",
            "--verbose",
            "-z",
            "--stdin",
        ],
        input=path.encode("utf-8") + b"\0",
        capture_output=True,
    )
    if result.returncode not in (0, 1):
        raise RuntimeError("Git ignore check failed")
    if result.returncode == 1:
        return False
    source, _, pattern, _ = result.stdout.rstrip(b"\0").split(b"\0")
    return source.decode("utf-8") in tracked_paths and not pattern.startswith(b"!")


def staged_entries():
    entries = []
    for record in git("ls-files", "--stage", "-z").split(b"\0"):
        if not record:
            continue
        metadata, raw_path = record.split(b"\t", 1)
        mode, blob, stage = metadata.decode("ascii").split()
        entries.append((raw_path.decode("utf-8"), mode, blob, stage))
    return entries


def forbidden_path(path):
    parts = PurePosixPath(path).parts
    folded = path.casefold()
    if folded.startswith(("analysis/", "docs/images/")) or folded in LOCAL_PATHS:
        return "upstream local reference or automation"
    if any(part.casefold() in FORBIDDEN_DIRECTORY_NAMES for part in parts):
        return "local-only directory"
    if PurePosixPath(path).name.casefold() in FORBIDDEN_FILE_NAMES:
        return "local-only instruction file"
    return None


def check_binaries(texts, blobs, binary_paths, errors):
    registrations = {}
    if EXCEPTIONS in texts:
        try:
            document = json.loads(texts[EXCEPTIONS])
            entries = document["binaries"]
            if not isinstance(entries, list):
                raise ValueError("expected a list")
            for entry in entries:
                path = entry.get("path") if isinstance(entry, dict) else None
                if (
                    not isinstance(path, str) or path not in blobs
                    or path in registrations or any(mark in path for mark in "*?[]")
                    or any(not isinstance(entry.get(key), str) or not entry[key].strip()
                           for key in ("reason", "source", "license", "sha256"))
                    or not re.fullmatch(r"[0-9a-f]{64}", entry["sha256"])
                    or type(entry.get("bytes")) is not int or entry["bytes"] < 1
                ):
                    raise ValueError("invalid entry")
                registrations[path] = entry
        except (KeyError, TypeError, ValueError):
            errors.append(f"{EXCEPTIONS}: invalid binary registration")
    for path in binary_paths:
        if path not in registrations:
            errors.append(f"{path}: unregistered binary or invalid UTF-8/NUL")
    for path, entry in registrations.items():
        data = blobs[path]
        if len(data) != entry["bytes"] or hashlib.sha256(data).hexdigest() != entry["sha256"]:
            errors.append(f"{path}: binary registration mismatch with staged bytes")


def check_source_records(texts, blobs, errors):
    manifest_path = "third_party/manifests/upstreams.json"
    if manifest_path in texts:
        try:
            manifest = json.loads(texts[manifest_path])
            for source in manifest["sources"]:
                if not re.fullmatch(r"[0-9a-f]{40}", source["revision"]):
                    raise ValueError("invalid source revision")
                for record in source.get("trackedEvidence", []):
                    data = blobs[record["path"]]
                    if len(data) != record["bytes"] or hashlib.sha256(data).hexdigest() != record["sha256"]:
                        errors.append(f"{manifest_path}: tracked source evidence mismatch")
                archive = source.get("gitArchive")
                if archive:
                    data = git("archive", "--format=tar", source["revision"])
                    if len(data) != archive["bytes"] or hashlib.sha256(data).hexdigest() != archive["sha256"]:
                        errors.append(f"{manifest_path}: upstream archive mismatch")
        except (KeyError, TypeError, ValueError):
            errors.append(f"{manifest_path}: invalid source registration")
    lock_record = "third_party/manifests/cargo-lock.json"
    if lock_record in texts:
        try:
            record = json.loads(texts[lock_record])
            lock_data = blobs["Cargo.lock"]
            packages = tomllib.loads(texts["Cargo.lock"])["package"]
            expected = sorted((package["name"], package["version"], package.get("checksum", ""))
                              for package in packages)
            registered = sorted((package["name"], package["version"], package.get("checksum", ""))
                                for package in record["packages"])
            if hashlib.sha256(lock_data).hexdigest() != record["lockSha256"] or expected != registered:
                errors.append(f"{lock_record}: Cargo.lock registration mismatch")
        except (KeyError, TypeError, ValueError):
            errors.append(f"{lock_record}: invalid dependency registration")


def markdown_prose(text):
    lines = []
    fence = None
    for line in text.splitlines():
        marker = re.match(r"^\s{0,3}(`{3,}|~{3,})", line)
        if marker:
            if fence is None:
                fence = marker[1]
            elif marker[1][0] == fence[0] and len(marker[1]) >= len(fence):
                fence = None
            continue
        if fence is None:
            lines.append(re.sub(r"(`+).*?\1", "", line))
    return "\n".join(lines)


def check():
    root = Path(git("rev-parse", "--show-toplevel").decode("utf-8").strip())
    if Path.cwd().resolve() != root.resolve():
        raise RuntimeError("Run the checker from the repository root")

    entries = staged_entries()
    errors = []
    texts = {}
    blobs = {}
    binary_paths = set()
    case_names = {}
    paths = {entry[0] for entry in entries}
    targets = set(paths)

    if not entries:
        errors.append("empty Git index: stage the intended files first")

    for path, mode, blob, stage in entries:
        parts = PurePosixPath(path).parts
        for offset in range(1, len(parts) + 1):
            prefix = "/".join(parts[:offset])
            targets.add(prefix)
            previous = case_names.setdefault(prefix.casefold(), prefix)
            if previous != prefix:
                errors.append(f"{path}: case collision with {previous}")

        for part in parts:
            if (
                RESERVED.match(part)
                or part.endswith((" ", "."))
                or re.search(r'[<>:"\\|?*\x00-\x1f]', part)
            ):
                errors.append(f"{path}: Windows-incompatible path")

        boundary = forbidden_path(path)
        if boundary:
            errors.append(f"{path}: forbidden {boundary}")
        if ignored(path, paths):
            errors.append(f"{path}: ignored path is tracked")

        if mode == "160000":
            errors.append(f"{path}: unregistered submodule; review source and pin before enabling")
            continue
        if mode not in ("100644", "100755") or stage != "0":
            errors.append(f"{path}: unsupported mode or unresolved index stage")
            continue

        size = int(git("cat-file", "-s", blob))
        if size > MAX_BYTES:
            errors.append(f"{path}: exceeds 10 MiB file limit")
            continue
        data = git("cat-file", "blob", blob)
        blobs[path] = data
        searchable = data.decode("utf-8", errors="ignore")
        if CREDENTIAL.search(searchable):
            errors.append(f"{path}: credential pattern detected (value withheld)")
        if MACHINE_PATH.search(searchable):
            errors.append(f"{path}: machine-private path detected (value withheld)")
        try:
            text = data.decode("utf-8")
        except UnicodeDecodeError:
            binary_paths.add(path)
            continue

        if "\x00" in text:
            binary_paths.add(path)
            continue
        if text.startswith("\ufeff"):
            errors.append(f"{path}: expected UTF-8 without BOM")
        texts[path] = text
        if path.endswith(".json"):
            try:
                json.loads(text)
            except (ValueError, RecursionError):
                errors.append(f"{path}: invalid JSON")

    check_binaries(texts, blobs, binary_paths, errors)
    check_source_records(texts, blobs, errors)
    ignore_files_match = True
    relevant_ignores = {".gitignore"}
    for path in paths:
        relevant_ignores.update(
            str(parent / ".gitignore") for parent in PurePosixPath(path).parents
        )
    for path in relevant_ignores - paths:
        if (root / path).is_file():
            errors.append(f"{path}: untracked .gitignore affects staged paths")
            ignore_files_match = False
    for path in paths:
        if PurePosixPath(path).name != ".gitignore":
            continue
        staged = texts.get(path)
        worktree = root / path
        if staged is None or not worktree.is_file() or worktree.read_text(encoding="utf-8") != staged:
            errors.append(f"{path} differs from staged .gitignore")
            ignore_files_match = False

    rules = [
        line.strip()
        for line in texts.get(".gitignore", "").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    ]
    if not rules or rules[-1] != "**/_temp/":
        errors.append("ignore boundary: the last active root rule must be **/_temp/")
    if ignore_files_match:
        probes = (
            "docs/HANDOFF.md",
            "docs/images/probe.png",
            "analysis/probe.md",
            "AGENTS.md",
            "nested/CLAUDE.md",
            ".agents/probe.txt",
            "nested/.agents/probe.txt",
            "nested/.claude/probe.txt",
            "output/probe.txt",
            "nested/output/probe.txt",
            "_temp/probe.txt",
            "references/_temp/probe.txt",
            ".local/private.txt",
            "nested/.local/private.txt",
            "node_modules/package/index.js",
            ".github/workflows/ci.yml",
            ".github/workflows/telegram-bot.yml",
            ".github/workflows/telegram-notify.yml",
        )
        for probe in probes:
            if not ignored(probe, paths):
                errors.append(f"ignore boundary: {probe} is exposed")

    for path, text in texts.items():
        if not path.endswith(".md"):
            continue
        for raw in LINK.findall(markdown_prose(text)):
            destination = raw[1 : raw.index(">")].strip() if raw.startswith("<") and ">" in raw else raw.split()[0]
            url = urlsplit(destination)
            if url.scheme or url.netloc or not url.path:
                continue
            decoded = unquote(url.path)
            resolved = posixpath.normpath(posixpath.join(posixpath.dirname(path), decoded))
            if decoded.startswith("/") or resolved == ".." or resolved.startswith("../") or resolved not in targets:
                errors.append(f"{path}: missing tracked link: {destination}")

    if errors:
        print("Repository check failed:")
        for error in sorted(set(errors)):
            print("- " + error)
        return 1
    print(f"Repository check passed: {len(paths)} staged files.")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(check())
    except (OSError, UnicodeError, RuntimeError, ValueError) as exc:
        print(f"Repository check could not complete: {exc}", file=sys.stderr)
        sys.exit(1)
