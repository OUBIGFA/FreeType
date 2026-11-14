"""Collect notices from the locked Windows dependencies and verified inputs."""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
import zipfile

from make_default_data import TOOL_INPUTS, checked_download, sha256, source_snapshot


ROOT = Path(__file__).resolve().parents[1]
APP = ROOT / "windows" / "app"
SOURCE_EXTENSIONS = {".go", ".rs", ".py", ".js", ".ts", ".c", ".cpp", ".h", ".nsh", ".nsi"}
NOTICE_NAME = re.compile(r"^(license|licence|copying|notice|copyright|patents|authors)(?:$|[._-])", re.I)


def json_stream(text):
    decoder = json.JSONDecoder()
    remaining = text.lstrip()
    while remaining:
        value, end = decoder.raw_decode(remaining)
        yield value
        remaining = remaining[end:].lstrip()


def run(arguments, cwd=ROOT):
    return subprocess.run(arguments, cwd=cwd, check=True, capture_output=True,
                          text=True, encoding="utf-8").stdout.strip()


def notice_files(source):
    return sorted(path for path in source.rglob("*")
                  if path.is_file() and not path.is_symlink() and NOTICE_NAME.match(path.name)
                  and path.suffix.lower() not in SOURCE_EXTENSIONS)


def needs_mpl_source(paths):
    return any(re.search(rb"Mozilla\s+Public\s+License", path.read_bytes(), re.I) for path in paths)


def copy_component(output, identifier, version, source, notice_paths=None, source_archive=None):
    paths = [source / name for name in notice_paths] if notice_paths is not None else notice_files(source)
    if not paths or any(not path.is_file() for path in paths):
        raise ValueError(f"Missing license files for {identifier}")
    mpl_source = needs_mpl_source(paths)
    cc_license = any(b"Attribution-ShareAlike 4.0 International" in path.read_bytes() for path in paths)
    if not (mpl_source or cc_license) and not any(re.search(r"license|licence|copying|copyright", path.name, re.I) for path in paths):
        raise ValueError(f"No license text for {identifier}; attribution alone is insufficient")
    if mpl_source and (source_archive is None or not source_archive.is_file()):
        raise ValueError(f"MPL corresponding source is required for {identifier}")
    if source_archive is not None and not source_archive.is_file():
        raise ValueError(f"Missing corresponding source for {identifier}")
    destination = output / (identifier.replace(":", "/", 1) + "@" + version)
    component = {"id": identifier, "version": version, "licenseFiles": [], "sourceFiles": []}
    for path in paths:
        if not path.resolve().is_relative_to(source.resolve()):
            raise ValueError(f"License escapes its source directory: {path.name}")
        target = destination / path.relative_to(source)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, target)
        component["licenseFiles"].append(target.relative_to(output).as_posix())
    if source_archive is not None:
        target = destination / "source" / source_archive.name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source_archive, target)
        component["sourceFiles"].append(target.relative_to(output).as_posix())
    return component


def collect_rust(output):
    selected = set()
    targets = [("x86_64-pc-windows-msvc", ["freetype-server", "freetype-tsf"]),
               ("i686-pc-windows-msvc", ["freetype-tsf"])]
    for target, packages in targets:
        arguments = ["cargo", "tree", "--locked", "--target", target,
                     "--edges", "normal", "--prefix", "none", "--format", "{p}"]
        for package in packages:
            arguments.extend(["-p", package])
        for line in run(arguments).splitlines():
            match = re.match(r"(\S+) v(\S+)", line)
            if match:
                selected.add(match.groups())
    metadata = json.loads(run(["cargo", "metadata", "--locked", "--format-version", "1",
                               "--filter-platform", "x86_64-pc-windows-msvc"]))
    components = []
    for package in sorted(metadata["packages"], key=lambda item: (item["name"], item["version"])):
        if package["source"] is None or (package["name"], package["version"]) not in selected:
            continue
        source = Path(package["manifest_path"]).parent
        component = copy_component(output, "rust:" + package["name"], package["version"], source)
        component["licenseExpression"] = package["license"]
        component["sourceUrl"] = package["repository"]
        components.append(component)
    return components


def collect_go(output):
    modules = {}
    for package in json_stream(run(["go", "list", "-deps", "-json=Module", "."], APP)):
        module = package.get("Module")
        if module and not module.get("Main"):
            if module.get("Replace"):
                raise ValueError("Review notices before using replaced Go modules")
            modules[module["Path"]] = module
    components = []
    for module in sorted(modules.values(), key=lambda item: item["Path"]):
        source = Path(module["Dir"])
        archive = None
        if needs_mpl_source(notice_files(source)):
            download = json.loads(run(["go", "mod", "download", "-json",
                                       module["Path"] + "@" + module["Version"]], APP))
            archive = Path(download["Zip"])
        component = copy_component(output, "go:" + module["Path"], module["Version"], source,
                                   source_archive=archive)
        component["sourceUrl"] = "https://pkg.go.dev/" + module["Path"] + "@" + module["Version"]
        components.append(component)
    return components, modules


def collect_nsis(output, archive, version):
    """Read only known notice files, even before mygo downloads its compiler."""
    destination = output / ("installer/nsis@" + version)
    component = {"id": "installer:nsis", "version": version, "licenseFiles": [], "sourceFiles": []}
    with tarfile.open(archive) as snapshot:
        for relative in ("COPYING", "Contrib/Modern UI 2/License.txt"):
            member = snapshot.getmember("nsis-" + version + "-src/" + relative)
            if not member.isfile():
                raise ValueError("NSIS notice is not a regular file: " + relative)
            target = destination / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            with snapshot.extractfile(member) as content:
                target.write_bytes(content.read())
            component["licenseFiles"].append(target.relative_to(output).as_posix())
    target = destination / "source" / archive.name
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(archive, target)
    component["sourceFiles"].append(target.relative_to(output).as_posix())
    return component


def collect_go_runtime(output, go_root, version):
    paths = [go_root / name for name in ("LICENSE", "PATENTS", "AUTHORS") if (go_root / name).is_file()]
    paths.extend(notice_files(go_root / "src/vendor"))
    return copy_component(output, "toolchain:go", version, go_root,
                          [path.relative_to(go_root) for path in paths])


def check_authorization(root=ROOT):
    record = json.loads((root / "third_party/manifests/authorization.json").read_text(encoding="utf-8"))
    code = record["wowtype"]
    for evidence in [code["readme"], code["package"], record["wordModel"]["notice"]]:
        if sha256(root / evidence["file"]) != evidence["sha256"]:
            raise ValueError("Upstream authorization evidence changed: " + evidence["file"])
    readme = (root / code["readme"]["file"]).read_text(encoding="utf-8")
    package = (root / code["package"]["file"]).read_text(encoding="utf-8")
    if code["declaration"] not in readme or ('license = "' + code["licenseExpression"] + '"') not in package:
        raise ValueError("Upstream authorization declaration does not match its evidence")
    if code["distributionLicense"] not in code["licenseExpression"].split(" OR "):
        raise ValueError("Selected distribution license is not authorized upstream")
    model = json.loads((root / "third_party/manifests/default-data.json").read_text(encoding="utf-8"))["languageModel"]
    if model["sha256"] != record["wordModel"]["sha256"] or model["license"] != record["wordModel"]["license"]:
        raise ValueError("Model changed; review its authorization before distribution")
    if sha256(root / record["unicode"]["source"]) != record["unicode"]["sourceSha256"]:
        raise ValueError("Unicode data changed; update its source and license record")
    return record


def archive_default_sources(archive, sources, snapshots, model_license):
    """Provide editable data, original headers and the exact conversion tools."""
    files = {}
    for source, snapshot in zip(sources, snapshots):
        selected = set(source.get("dictionaries", []))
        for path in sorted(snapshot.rglob("*")):
            if not path.is_file():
                continue
            relative = path.relative_to(snapshot).as_posix()
            notice = relative in {"LICENSE", "LICENSE-SCOPE.md", "NOTICE.md"}
            if not notice and (not relative.endswith(".dict.yaml") or (selected and relative not in selected)):
                continue
            files[source["name"] + "/" + relative] = path
    for name in [*TOOL_INPUTS, "data/hotwords.tsv", "third_party/manifests/default-data.json",
                 "third_party/licenses/Apache-2.0-LICENSE.txt", "third_party/licenses/WowType-README.txt"]:
        files[name] = ROOT / name
    files["licenses/CC-BY-SA-4.0.txt"] = model_license
    record = {"format": 1, "sources": sources,
              "files": {name: sha256(path) for name, path in sorted(files.items())}}
    instructions = (
        "FreeType 默认数据的对应源码\n\n"
        "保留原始词库文件、署名、许可范围及转换脚本；未包含词库上游的程序代码。\n"
        "解压后安装 Python 3.11 或更新版本，在本目录运行：\n"
        'python -X utf8 tools/make_default_data.py --snapshot-root . --model "已安装的 data/words.lm" --output rebuilt\n'
        "模型由原发布方按 CC BY-SA 4.0 单独发布，使用安装包内的原文件即可，无须下载训练语料。\n"
        "source-manifest.json 记录原始文件摘要；修改输入后应明确更新其记录与版本，不冒充原始快照。\n"
        "各来源保留各自许可；此归档不把独立程序、模型及数据统一改成一种许可。\n"
    )
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as bundle:
        for name, path in sorted(files.items()):
            entry = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            entry.compress_type = zipfile.ZIP_DEFLATED
            bundle.writestr(entry, path.read_bytes())
        for name, content in {"source-manifest.json": json.dumps(record, ensure_ascii=False, indent=2) + "\n",
                              "README.txt": instructions}.items():
            entry = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            entry.compress_type = zipfile.ZIP_DEFLATED
            bundle.writestr(entry, content.encode("utf-8"))


def collect(output):
    inputs = json.loads((ROOT / "third_party/manifests/notice-inputs.json").read_text(encoding="utf-8"))
    for license_text in inputs["licenseTexts"]:
        if sha256(ROOT / license_text["file"]) != license_text["sha256"]:
            raise ValueError("License text no longer matches its recorded source: " + license_text["file"])
    authorization = check_authorization()
    run(["go", "mod", "verify"], APP)
    components = collect_rust(output)
    go_components, modules = collect_go(output)
    components.extend(go_components)

    go_root = Path(run(["go", "env", "GOROOT"], APP))
    components.append(collect_go_runtime(output, go_root, run(["go", "env", "GOVERSION"], APP)))
    rust_root = Path(run(["rustc", "--print", "sysroot"])) / "share/doc/rust"
    rust_version = run(["rustc", "--version"]).split()[1]
    components.append(copy_component(output, "toolchain:rust", rust_version, rust_root,
                                     ["COPYRIGHT-library.html"]))

    cache = ROOT / ".local/third-party"
    suffix = inputs["publicSuffix"]
    module = modules[suffix["module"]]
    table = Path(module["Dir"]) / "publicsuffix/table.go"
    if module["Version"] != suffix["moduleVersion"] or suffix["revision"] not in table.read_text(encoding="utf-8"):
        raise ValueError("Update the public-suffix source notice for the selected Go module")
    suffix_source = checked_download(cache / ("public-suffix-list-" + suffix["revision"] + ".dat"),
                                     suffix["url"], suffix["sha256"])
    components.append(copy_component(output, "data:public-suffix-list", suffix["revision"],
                                     ROOT / "third_party/licenses", ["MPL-2.0.txt"], suffix_source))

    nsis_source = inputs["nsisSource"]
    archive = checked_download(cache / ("nsis-" + nsis_source["version"] + "-src.tar.bz2"),
                               nsis_source["url"], nsis_source["sha256"])
    nsis_component = collect_nsis(output, archive, nsis_source["version"])
    nsis_component["sourceUrl"] = nsis_source["url"]
    components.append(nsis_component)
    components.append(copy_component(output, "asset:octicons", "68ec2de8d0ddfcbb7497da01d0b7d98eb5593020",
                                     APP / "assets", ["octicons-license.txt"]))
    components.append(copy_component(output, "data:opencc", "wowtype-5656e136",
                                     ROOT / "third_party/licenses", ["OpenCC-LICENSE.txt", "OpenCC-AUTHORS.txt"],
                                     ROOT / "core/data/s2t.tsv"))
    licenses = ROOT / "third_party/licenses"
    components.append(copy_component(output, "engine:wowtype-core", authorization["wowtype"]["revision"], licenses,
                                     ["Apache-2.0-LICENSE.txt", "WowType-README.txt", "WowType-Cargo.toml.txt"]))
    components.append(copy_component(output, "data:unicode-cldr", "wowtype-5656e136", licenses,
                                     ["Unicode-LICENSE.txt"], ROOT / authorization["unicode"]["source"]))
    model_notice = copy_component(output, "model:word-lm", authorization["wordModel"]["tag"],
                                  ROOT / ".local/data-src/archives", ["CC-BY-SA-4.0.txt"])
    model_target = output / ("model/word-lm@" + authorization["wordModel"]["tag"]) / "SOURCE.txt"
    shutil.copyfile(ROOT / authorization["wordModel"]["notice"]["file"], model_target)
    model_notice["licenseFiles"].append(model_target.relative_to(output).as_posix())
    components.append(model_notice)
    data = ROOT / ".local/data"
    parent = Path(os.environ.get("FREETYPE_TEST_TEMP", ROOT / "_temp/notices"))
    parent.mkdir(parents=True, exist_ok=True)
    scratch = Path(tempfile.mkdtemp(prefix="data-sources-", dir=parent))
    data_manifest = json.loads((ROOT / "third_party/manifests/default-data.json").read_text(encoding="utf-8"))
    archives = ROOT / ".local/data-src/archives"
    snapshots = [source_snapshot(source, archives, scratch) for source in data_manifest["sources"]]
    data_sources = scratch / "default-data-sources.zip"
    archive_default_sources(data_sources, data_manifest["sources"], snapshots, archives / "CC-BY-SA-4.0.txt")
    components.append(copy_component(output, "data:default", (data / "VERSION").read_text(encoding="utf-8").strip(),
                                     data, ["LICENSE", "SOURCE.txt", "WANXIANG-LICENSE-SCOPE.md", "WANXIANG-NOTICE.md"],
                                     data_sources))
    manifest = {
        "format": 1,
        "scope": "Windows normal Rust dependency graphs, imported Go modules, bundled assets and runtime notices; not a symbol-level linkage inventory",
        "components": sorted(components, key=lambda item: (item["id"], item["version"])),
        "authorizationRecordSha256": sha256(ROOT / "third_party/manifests/authorization.json"),
        "unresolved": [],
        "files": {path.relative_to(output).as_posix(): sha256(path)
                  for path in sorted(output.rglob("*")) if path.is_file()},
    }
    (output / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
                                          encoding="utf-8", newline="\n")
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path)
    arguments = parser.parse_args()
    if arguments.out.exists():
        parser.error("Use a new output directory; existing notices are not overwritten")
    manifest = collect(arguments.out)
    print(f"Collected {len(manifest['components'])} components and {len(manifest['files'])} notice/source files; "
          f"{len(manifest['unresolved'])} upstream permission issue remains")


if __name__ == "__main__":
    main()
