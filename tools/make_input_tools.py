"""Build complete, fixed-source English and decomposition lookup tables for Windows."""
import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import unicodedata
import shutil
import zlib

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "third_party/manifests/input-tools.json"


def clean(text):
    return str(text or "").replace("\\n", "；").replace("\r", " ").replace("\n", "；").replace("\t", " ").replace("\0", "").strip()


def spelling_key(word):
    """A small phonetic bucket, never an automatic correction: candidates still need edit distance."""
    word = word.lower().replace("ph", "f").replace("ck", "k").replace("qu", "k").replace("gh", "g")
    groups = {c: value for chars, value in [("bpfv", "b"), ("cgjkqsxz", "k"), ("dt", "t"), ("mn", "n"), ("l", "l"), ("r", "r")] for c in chars}
    result = []
    for ch in word:
        value = groups.get(ch)
        if value and (not result or value != result[-1]):
            result.append(value)
    return "".join(result) or "a"


def english_entries(rows):
    for row in rows:
        word = clean(row.get("word"))
        # Keep complete lexical entries and phrases, not affixes, formulae or numeric noise.
        if not word or word.startswith("-") or len(word) > 160 or any(not (ch.isascii() and (ch.isalpha() or ch in " '-.")) for ch in word):
            continue
        key = "".join(ch for ch in word.lower() if "a" <= ch <= "z")
        if not key or len(key) > 100:
            continue
        frequencies = [int(row.get(field) or 0) for field in ("bnc", "frq") if str(row.get(field) or "0").isdigit()]
        ranks = [rank for rank in frequencies if rank > 0]
        rank = 2_000_000 - min(min(ranks), 1_999_999) if ranks else 0
        yield key, rank, word, clean(row.get("phonetic")), clean(row.get("translation"))


def read_pinyin(path):
    result = {}
    for line in path.read_text(encoding="utf-8-sig").splitlines():
        match = re.match(r"U\+([0-9A-F]+):\s*([^#]+)", line)
        if match:
            result[chr(int(match[1], 16))] = [v.strip() for v in match[2].split(",") if v.strip()]
    return result


def radical_entries(lines, readings, modern):
    entries = {}
    def chosen_readings(word):
        all_readings = readings.get(word, [])
        chosen = all_readings[:1]
        dictionaries = [dictionary[word] for dictionary in modern if word in dictionary]
        for value in all_readings[1:]:
            if not dictionaries or sum(value in values for values in dictionaries) >= min(2, len(dictionaries)):
                chosen.append(value)
                break
        return chosen
    for line in lines:
        if line.startswith("#") or "\t" not in line:
            continue
        columns = line.rstrip().split("\t")
        if len(columns) < 2:
            continue
        word, parts = columns[:2]
        if len(word) != 1 or not ("\u3400" <= word <= "\u9fff" or "\uf900" <= word <= "\ufaff" or 0x20000 <= ord(word) <= 0x323AF):
            continue
        key = parts.replace("'", "").replace(" ", "")
        if not key or any(not "a" <= c <= "z" for c in key):
            continue
        weight = int(columns[2]) if len(columns) > 2 and columns[2].isdigit() else 0
        chosen = chosen_readings(word)
        entry = key, weight, word, " / ".join(chosen), ""
        previous = entries.get((key, word))
        if previous is None or weight > previous[1]:
            entries[key, word] = entry
    # A component may itself be the requested character (kou -> 口). Include
    # its documented modern readings without inventing per-character exceptions.
    weights = {}
    for _, weight, word, _, _ in entries.values():
        weights[word] = max(weights.get(word, 0), weight)
    for word in weights:
        chosen = chosen_readings(word)
        for reading in chosen:
            plain = unicodedata.normalize("NFD", reading).replace("u\u0308", "v")
            key = "".join(c for c in plain if "a" <= c <= "z")
            if key:
                entries.setdefault((key, word), (key, weights[word], word, " / ".join(chosen), ""))
    yield from entries.values()


def encode_table(entries):
    """Offsets and CRC allow a read-only mapped reader to validate before serving queries."""
    rows = sorted(set(entries), key=lambda row: (row[0], -row[1], row[2], row[3], row[4]))
    records = [("\t".join([key, str(rank), clean(word), clean(reading), clean(gloss)]) + "\n").encode("utf-8") for key, rank, word, reading, gloss in rows]
    offset = 16 + 8 * (len(records) + 1)
    offsets = [offset]
    for record in records:
        offset += len(record)
        offsets.append(offset)
    body = struct.pack(f"<{len(offsets)}Q", *offsets) + b"".join(records)
    return b"FTLOOK01" + struct.pack("<II", len(records), zlib.crc32(body)) + body


def digest(path):
    return hashlib.file_digest(path.open("rb"), "sha256").hexdigest()


def prepare_sources(manifest):
    base = ROOT / ".local/data-src/input-tools"
    for source in manifest["sources"]:
        for item in source["files"]:
            path = base / source["name"] / item["name"]
            if not path.is_file() or path.with_name(path.name + ".aria2").exists():
                path.parent.mkdir(parents=True, exist_ok=True)
                downloader = os.environ.get("ARIA2_NEXT") or shutil.which("aria2-next")
                if not downloader:
                    raise RuntimeError("需要 aria2-next；设置 ARIA2_NEXT 为其完整路径")
                subprocess.run([downloader, "-c", "--auto-file-renaming=false", "-d", str(path.parent), "-o", path.name, item["url"]], check=True)
            if path.stat().st_size != item["bytes"] or digest(path) != item["sha256"]:
                raise ValueError(f"输入数据校验失败：{source['name']}/{item['name']}")
    return base


def check(output):
    record = json.loads((output / "INPUT-TOOLS.json").read_text(encoding="utf-8"))
    if record["manifestSha256"] != digest(MANIFEST) or record["generatorSha256"] != digest(Path(__file__)):
        raise ValueError("英文与拆字数据输入已变化，请重新生成")
    for name, entry in record["files"].items():
        if digest(output / name) != entry["sha256"]:
            raise ValueError(f"英文与拆字数据损坏：{name}")
    return record


def build(output):
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    source = prepare_sources(manifest)
    with (source / "ecdict/ecdict.csv").open(encoding="utf-8-sig", newline="") as handle:
        english = list(english_entries(csv.DictReader(handle)))
    readings = read_pinyin(source / "pinyin/pinyin.txt")
    modern = [read_pinyin(source / "pinyin" / name) for name in ("kHanyuPinlu.txt", "kXHC1983.txt", "kTGHZ2013.txt")]
    radical = list(radical_entries((source / "radical/radical_pinyin.dict.yaml").read_text(encoding="utf-8").splitlines(), readings, modern))
    if len(english) < manifest["minimumEnglishEntries"] or len(radical) < manifest["minimumRadicalEntries"]:
        raise ValueError("生成的输入数据不完整，未交付演示词表")
    spelling = [(spelling_key(word), rank, word, "", "") for _, rank, word, _, _ in english if " " not in word and "-" not in word]
    output.mkdir(parents=True, exist_ok=True)
    files = {}
    for name, entries in [("english-lookup.ftd", english), ("english-spelling.ftd", spelling), ("radical-lookup.ftd", radical)]:
        data = encode_table(entries)
        temporary = output / (name + ".tmp")
        temporary.write_bytes(data)
        temporary.replace(output / name)
        files[name] = {"entries": struct.unpack_from("<I", data, 8)[0], "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}
    record = {"version": manifest["version"], "manifestSha256": digest(MANIFEST), "generatorSha256": digest(Path(__file__)), "files": files}
    (output / "INPUT-TOOLS.json").write_text(json.dumps(record, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return check(output)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--output", type=Path, default=ROOT / ".local/data")
    args = parser.parse_args()
    print(json.dumps(check(args.output) if args.check else build(args.output), ensure_ascii=False, indent=2))
