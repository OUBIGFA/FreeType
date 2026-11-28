#!/usr/bin/env python3
"""Build the weekly FreeType dictionary update package from the latest approved upstream data.

resolve: pin the newest upstream commits of the approved sources, verify their licenses
         are unchanged and print the input digest used to skip unchanged weeks.
build:   rebuild the data with the existing make_default_data.py rules, compile the lexicon
         with FreeTypeServer, check the package by actually loading it and write the
         release assets (zip, JSON manifest and release notes).

Only data from the whitelisted sources is used; no upstream scripts are executed.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
from make_default_data import DEFAULT_MANIFEST, ROOT, TOOL_INPUTS, checked_download, resolve_data_inputs, sha256  # noqa: E402

CHANNEL = ROOT / "third_party" / "manifests" / "dictionary-channel.json"
HOTWORDS = ROOT / "data" / "hotwords.tsv"
# 中文主词库的转换与编译代码；它们变化时即使上游数据未变也要重新生成。
CONVERTER_TREES = ("core/src", "tools")


def git(*args):
    return subprocess.run(["git", *args], cwd=ROOT, check=True, capture_output=True, text=True).stdout.strip()


def valid_version(version):
    parts = version.split(".")
    return len(parts) == 4 and all(p.isdigit() for p in parts) and len(parts[0]) == 4


def latest_revision(repository, ref):
    line = git("ls-remote", repository, ref).splitlines()
    if not line:
        raise ValueError(f"无法取得上游修订：{repository} {ref}")
    revision = line[0].split()[0]
    if len(revision) != 40:
        raise ValueError(f"上游修订格式异常：{repository}")
    return revision


def archive_licenses(archive, name, revision):
    found = {}
    with tarfile.open(archive, "r:gz") as bundle:
        for member in bundle:
            prefix = f"{name}-{revision}/"
            if member.isfile() and member.name.startswith(prefix) and "/" not in member.name[len(prefix):]:
                found[member.name[len(prefix):]] = hashlib.sha256(bundle.extractfile(member).read()).hexdigest()
    return found


def resolve(args):
    channel = json.loads(CHANNEL.read_text(encoding="utf-8"))
    manifest = json.loads(DEFAULT_MANIFEST.read_text(encoding="utf-8"))
    archives = args.source_root / "archives"
    for source in manifest["sources"]:
        rules = channel["sources"].get(source["name"])
        if rules is None:
            raise ValueError(f"来源不在词库渠道白名单中：{source['name']}")
        revision = latest_revision(source["repository"], rules["ref"])
        archive = checked_download(
            archives / f"{source['name']}-{revision}.tar.gz",
            f"https://codeload.github.com/{source['repository'].removeprefix('https://github.com/')}/tar.gz/{revision}",
        )
        licenses = archive_licenses(archive, source["name"], revision)
        for license_name, expected in rules["licenses"].items():
            if licenses.get(license_name) != expected:
                # 许可或范围说明变化需要人工核对后更新白名单，本轮停止发布。
                raise ValueError(f"{source['name']} 的 {license_name} 已变化，停止本轮词库发布")
        source["revision"] = revision
        source["archiveSha256"] = sha256(archive)
    manifest["version"] = args.version
    args.work.mkdir(parents=True, exist_ok=True)
    resolved = args.work / "default-data.json"
    resolved.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    blocklist, _, corpus = resolve_data_inputs(manifest)
    inputs = {
        "channel": sha256(CHANNEL),
        "sources": [[s["name"], s["revision"], s["archiveSha256"]] for s in manifest["sources"]],
        "rules": {k: v for k, v in manifest.items() if k not in ("version", "sources")},
        "hotwords": sha256(HOTWORDS),
        "blocklist": sha256(blocklist) if blocklist else "",
        "corpus": sha256(corpus) if corpus else "",
        "tools": {name: sha256(ROOT / name) for name in TOOL_INPUTS + ("tools/make_dictionary_package.py",)},
        "converter": {tree: git("rev-parse", f"HEAD:{tree}") for tree in CONVERTER_TREES},
    }
    digest = hashlib.sha256(json.dumps(inputs, sort_keys=True).encode()).hexdigest()
    record = {"version": args.version, "inputsDigest": digest,
              "sources": [{"name": s["name"], "repository": s["repository"], "revision": s["revision"]} for s in manifest["sources"]]}
    (args.work / "resolved.json").write_text(json.dumps(record, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(json.dumps(record, ensure_ascii=False))


def lexicon_format(path):
    with path.open("rb") as source:
        magic = source.read(4)
    if magic not in (b"FTD1", b"FTD2"):
        raise ValueError(f"编译出的词库格式无法识别：{magic!r}")
    return magic.decode()


def write_zip(path, directory, names):
    """Fixed order and timestamps keep identical inputs byte-identical."""
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as bundle:
        for name in sorted(names):
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            with (directory / name).open("rb") as source, bundle.open(info, "w") as target:
                for block in iter(lambda: source.read(1 << 20), b""):
                    target.write(block)


def build(args):
    if not valid_version(args.version):
        raise ValueError("词库版本应形如 2026.11.29.1")
    channel = json.loads(CHANNEL.read_text(encoding="utf-8"))
    resolved = json.loads((args.work / "resolved.json").read_text(encoding="utf-8"))
    if resolved["version"] != args.version:
        raise ValueError("resolve 与 build 的版本不一致")
    data = args.work / "data"
    command = [sys.executable, "-X", "utf8", str(ROOT / "tools" / "make_default_data.py"),
               "--manifest", str(args.work / "default-data.json"), "--source-root", str(args.source_root),
               "--output", str(data)]
    if args.model:
        command += ["--model", str(args.model)]
    subprocess.run(command, check=True)
    package = args.work / "package"
    package.mkdir(parents=True, exist_ok=False)
    subprocess.run([str(args.server), "--compile-lexicon", str(data / "lexicon.tsv"), str(package / "lexicon.ftd")], check=True)
    for name in ["english.txt"] + channel["extraFiles"]:
        (package / name).write_bytes((data / name).read_bytes())
    converter = git("rev-parse", "HEAD")
    with (package / "SOURCE.txt").open("a", encoding="utf-8", newline="\n") as notes:
        notes.write(f"词库更新包 {args.version}；转换器修订 {converter}；重建入口 tools/make_dictionary_package.py。\n")
    files = {name: {"bytes": (package / name).stat().st_size, "sha256": sha256(package / name)} for name in channel["files"]}
    record = json.loads((data / "MANIFEST.json").read_text(encoding="utf-8"))
    manifest = {
        "format": channel["packageFormat"], "channel": channel["channel"], "version": args.version,
        "lexiconFormat": lexicon_format(package / "lexicon.ftd"),
        "inputsDigest": resolved["inputsDigest"], "converter": converter,
        "sources": resolved["sources"], "counts": record["counts"], "files": files,
    }
    (package / "PACKAGE.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    # 实际载入：核对全部摘要、用全新个人数据目录启动引擎并确认代表性候选。
    subprocess.run([str(args.server), "--check-dictionary", str(package)], check=True)
    args.output.mkdir(parents=True, exist_ok=True)
    zip_name = f"FreeType-Dictionaries-{args.version}.zip"
    write_zip(args.output / zip_name, package, channel["files"] + channel["extraFiles"] + ["PACKAGE.json"])
    manifest["archive"] = {"name": zip_name, "bytes": (args.output / zip_name).stat().st_size,
                           "sha256": sha256(args.output / zip_name)}
    (args.output / f"FreeType-Dictionaries-{args.version}.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    counts = record["counts"]
    lines = [f"FreeType 词库更新包 {args.version}。", ""]
    lines += [f"- {s['name']}：`{s['revision']}`（{s['repository']}）" for s in resolved["sources"]]
    lines += [
        f"- 词条：{counts['entries']} 条；白霜 {counts['base']} 条，万象补缺 {counts['wanxiangAdded']} 条。",
        f"- 转换器修订：`{converter}`。",
        "",
        "在 FreeType 设置的「词库 → 词库更新」中检查更新或开启自动更新。下载校验后，下次服务启动时生效。",
        "本 Release 只含词库数据，不含程序安装器。许可与来源见压缩包内的 LICENSE 和 SOURCE.txt。",
    ]
    (args.output / "notes.md").write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print(json.dumps({"version": args.version, "files": {k: v["sha256"] for k, v in files.items()}}, ensure_ascii=False))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", choices=["resolve", "build"])
    parser.add_argument("--version", required=True)
    parser.add_argument("--work", type=Path, default=ROOT / "_temp" / "dictionary-package")
    parser.add_argument("--source-root", type=Path, default=ROOT / ".local" / "data-src")
    parser.add_argument("--server", type=Path, default=ROOT / "target" / "release" / "FreeTypeServer.exe")
    parser.add_argument("--model", type=Path, default=os.environ.get("WOWTYPE_LM_FILE"))
    parser.add_argument("--output", type=Path, default=ROOT / "output" / "dictionary-package")
    args = parser.parse_args()
    if not valid_version(args.version):
        parser.exit(1, "词库版本应形如 2026.11.29.1\n")
    try:
        resolve(args) if args.command == "resolve" else build(args)
    except (OSError, ValueError, KeyError, RuntimeError, subprocess.CalledProcessError, tarfile.TarError) as error:
        parser.exit(1, f"词库更新包未完成：{error}\n")


if __name__ == "__main__":
    main()
