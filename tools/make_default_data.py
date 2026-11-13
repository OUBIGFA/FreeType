#!/usr/bin/env python3
"""Build pinned FreeType data from Baishuang, Wanxiang and project hotwords."""

import argparse
import hashlib
import json
import lzma
import os
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import sys
import tarfile
import tempfile

from rime2tsv import add_extra_chars, load_entries


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_MANIFEST = ROOT / "third_party" / "manifests" / "default-data.json"
TOOL_INPUTS = ("tools/make_default_data.py", "tools/rime2tsv.py", "tools/add_hotwords.py")


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def checked_download(path, url, expected=None):
    if not path.is_file() or path.with_name(path.name + ".aria2").exists():
        downloader = os.environ.get("ARIA2_NEXT") or shutil.which("aria2-next")
        if not downloader or not Path(downloader).is_file():
            raise RuntimeError("需要 aria2-next；设置 ARIA2_NEXT 为其完整路径后重试。")
        path.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run([downloader, "-c", "--auto-file-renaming=false", "-d", str(path.parent),
                        "-o", path.name, url], check=True)
    if expected and sha256(path) != expected:
        raise ValueError(f"SHA-256 不匹配：{path.name}；请核对下载文件，未继续构建。")
    return path


def source_snapshot(source, archives, scratch):
    """Extract data only; Wanxiang's tools have a separate licensing boundary."""
    name = source["name"]
    revision = source["revision"]
    archive = checked_download(
        archives / f"{name}-{revision}.tar.gz",
        f"https://codeload.github.com/{source['repository'].removeprefix('https://github.com/')}/tar.gz/{revision}",
        source["archiveSha256"],
    )
    destination = scratch / name
    prefix = f"{name}-{revision}/"
    directories = {"cn_dicts", "cn_dicts_cell", "cn_dicts_common", "en_dicts", "dicts"}
    root_files = {"LICENSE", "LICENSE-SCOPE.md", "NOTICE.md", source.get("dictionary", "")}
    with tarfile.open(archive, "r:gz") as bundle:
        for member in bundle:
            if not member.isfile() or not member.name.startswith(prefix):
                continue
            relative = PurePosixPath(member.name.removeprefix(prefix))
            if relative.is_absolute() or ".." in relative.parts:
                raise ValueError(f"词库归档包含越界路径：{name}")
            if str(relative) not in root_files and not (
                relative.parts[0] in directories and str(relative).endswith(".dict.yaml")
            ):
                continue
            target = destination.joinpath(*relative.parts)
            target.parent.mkdir(parents=True, exist_ok=True)
            with bundle.extractfile(member) as incoming, target.open("wb") as outgoing:
                shutil.copyfileobj(incoming, outgoing)
    return destination


def scale_frequencies(entries, total):
    current = sum(entries.values())
    if total > 0 and current > 0:
        factor = total / current
        for key in entries:
            entries[key] *= factor


def input_record(manifest, hotwords):
    return {
        "configurationSha256": sha256(manifest),
        "hotwordsSha256": sha256(hotwords),
        "tools": {name: sha256(ROOT / name) for name in TOOL_INPUTS},
    }


def check_data(output, manifest_path, hotwords):
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    record = json.loads((output / "MANIFEST.json").read_text(encoding="utf-8"))
    if record["version"] != manifest["version"] or record["inputs"] != input_record(manifest_path, hotwords):
        raise ValueError("默认词库输入已变化；先运行 python -X utf8 tools/make_default_data.py。")
    expected = [(source["name"], source["revision"], source["archiveSha256"]) for source in manifest["sources"]]
    actual = [(source["name"], source["revision"], source["archiveSha256"]) for source in record["sources"]]
    if actual != expected:
        raise ValueError("默认词库来源与固定修订不一致，未继续打包。")
    for name, details in record["files"].items():
        path = output / name
        if path.stat().st_size != details["bytes"] or sha256(path) != details["sha256"]:
            raise ValueError(f"默认数据校验失败：{name}；重新生成数据后重试。")
    print(f"默认词库校验通过：{record['counts']['entries']} 条，版本 {record['version']}")
    return record


def build_data(args):
    manifest_path = args.manifest.resolve()
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    archives = args.source_root / "archives"
    archives.mkdir(parents=True, exist_ok=True)
    model_info = manifest["languageModel"]
    model = args.model or args.output / "words.lm"
    if model.is_file() and sha256(model) != model_info["sha256"]:
        raise ValueError("words.lm 的 SHA-256 不匹配，未替换已有词库。")
    model_license = checked_download(archives / "CC-BY-SA-4.0.txt", model_info["licenseUrl"],
                                     model_info["licenseSha256"])
    parent = Path(os.environ.get("FREETYPE_TEST_TEMP", ROOT / "_temp" / "default-data"))
    parent.mkdir(parents=True, exist_ok=True)
    scratch = Path(tempfile.mkdtemp(prefix="data-", dir=parent))
    sources = manifest["sources"]
    snapshots = [source_snapshot(source, archives, scratch) for source in sources]
    primary, supplement = sources
    frost, wanxiang = snapshots
    entries = load_entries(frost / primary["dictionary"], set(), skip=tuple(primary["skipImports"]),
                           aliases=primary["importAliases"])
    scale_frequencies(entries, manifest["frequencyTotal"])
    base_count = len(entries)
    additional = {}
    for name in supplement["dictionaries"]:
        for key, weight in load_entries(wanxiang / name, set()).items():
            additional[key] = max(additional.get(key, 0), weight)
    scale_frequencies(additional, manifest["frequencyTotal"])
    added_count = 0
    for key, weight in additional.items():
        if key not in entries:
            entries[key] = weight
            added_count += 1
    del additional
    extra_count = sum(add_extra_chars(entries, frost / name) for name in primary["extraChars"])
    if len(entries) < manifest["minimumEntries"]:
        raise ValueError("词库条数低于固定输入要求，未生成不完整数据。")
    raw = scratch / "merged.tsv"
    with raw.open("w", encoding="utf-8", newline="\n") as output:
        output.write("# FreeType：白霜主库、万象补充；拼音<TAB>词<TAB>词频\n")
        for (code, word), weight in sorted(entries.items(), key=lambda item: (-item[1], item[0])):
            formatted = f"{weight:.1f}" if weight < 1 else str(int(round(weight)))
            output.write(f"{code}\t{word}\t{formatted}\n")
    del entries
    built = scratch / "output"
    built.mkdir()
    with (built / "lexicon.tsv").open("w", encoding="utf-8", newline="\n") as output:
        subprocess.run([sys.executable, "-X", "utf8", str(ROOT / "tools" / "add_hotwords.py"),
                        str(raw), str(args.hotwords)], stdout=output, check=True)
    if not model.is_file():
        compressed = checked_download(archives / "words.lm.xz", model_info["url"])
        with lzma.open(compressed, "rb") as incoming, (built / "words.lm").open("wb") as outgoing:
            shutil.copyfileobj(incoming, outgoing)
        if sha256(built / "words.lm") != model_info["sha256"]:
            raise ValueError("下载的 words.lm 校验失败，未替换已有词库。")
    else:
        shutil.copyfile(model, built / "words.lm")
    english = "\n".join((frost / name).read_text(encoding="utf-8") for name in primary["english"])
    (built / "english.txt").write_text(english, encoding="utf-8", newline="\n")
    licenses = "\n\n".join([frost.joinpath("LICENSE").read_text(encoding="utf-8"),
                             wanxiang.joinpath("LICENSE").read_text(encoding="utf-8"),
                             model_license.read_text(encoding="utf-8")])
    (built / "LICENSE").write_text(licenses, encoding="utf-8", newline="\n")
    for name in ("LICENSE-SCOPE.md", "NOTICE.md"):
        shutil.copyfile(wanxiang / name, built / f"WANXIANG-{name}")
    descriptions = ["FreeType 默认数据：白霜完整主库＋项目热词＋万象核心词库。"]
    for source in sources:
        descriptions.append(f"来源：{source['repository']}；版本：{source['revision']}；许可：{source['license']}。")
    descriptions.extend([
        "万象数据作者：amzxyz；只使用数据，不复制其程序代码或 .gram 模型。",
        "万象表：" + ", ".join(supplement["dictionaries"]) + "。",
        "改动：拼音去声调，ü 转 v；同音同词保留白霜权重，万象只补缺；两库分别归一化词频。",
        "白霜的 extthot 引用修正为 exthot；两张大字表只补缺字，频次小于 1。",
        "不收白霜 corrections 和万象 cuoyin 错音错字表：FreeType 没有 Rime 的纠音提示。",
        "项目热词由 data/hotwords.tsv 合入，最低词频 30000。",
        "英文词表来自白霜；语言模型沿用 WowType words.lm（CC BY-SA 4.0）。",
        "模型来源：" + model_info["url"],
        "许可文本见 LICENSE；万象许可范围及第三方声明见 WANXIANG-LICENSE-SCOPE.md、WANXIANG-NOTICE.md。",
    ])
    (built / "SOURCE.txt").write_text("\n".join(descriptions) + "\n", encoding="utf-8", newline="\n")
    (built / "VERSION").write_text(manifest["version"] + "\n", encoding="utf-8", newline="\n")
    count = sum(1 for line in (built / "lexicon.tsv").open(encoding="utf-8") if not line.startswith("#"))
    record = {
        "version": manifest["version"],
        "sources": sources,
        "inputs": input_record(manifest_path, args.hotwords),
        "counts": {"base": base_count, "wanxiangAdded": added_count, "extraChars": extra_count, "entries": count},
        "files": {path.name: {"bytes": path.stat().st_size, "sha256": sha256(path)} for path in built.iterdir()},
    }
    (built / "MANIFEST.json").write_text(json.dumps(record, ensure_ascii=False, indent=2) + "\n",
                                         encoding="utf-8", newline="\n")
    args.output.mkdir(parents=True, exist_ok=True)
    for path in sorted(built.iterdir(), key=lambda item: item.name == "MANIFEST.json"):
        os.replace(path, args.output / path.name)
    check_data(args.output, manifest_path, args.hotwords)
    print(f"白霜 {base_count} 条，万象补缺 {added_count} 条；过程文件：{scratch}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--source-root", type=Path, default=ROOT / ".local" / "data-src")
    parser.add_argument("--output", type=Path, default=ROOT / ".local" / "data")
    parser.add_argument("--model", type=Path, default=os.environ.get("WOWTYPE_LM_FILE"))
    parser.add_argument("--hotwords", type=Path, default=ROOT / "data" / "hotwords.tsv")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    try:
        if args.check:
            check_data(args.output, args.manifest, args.hotwords)
        else:
            build_data(args)
    except (OSError, ValueError, KeyError, RuntimeError, subprocess.CalledProcessError, tarfile.TarError, lzma.LZMAError) as error:
        parser.exit(1, f"默认词库未完成：{error}\n")


if __name__ == "__main__":
    main()
