"""Collect notices from the locked Windows dependencies and verified inputs."""

import argparse
import hashlib
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
from rime2tsv import parse_header, resolve_import


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
    keyboard = record["wowtypeKeyboard"]
    for evidence in [keyboard["readme"], keyboard["source"]]:
        if sha256(root / evidence["file"]) != evidence["sha256"]:
            raise ValueError("Keyboard authorization evidence changed: " + evidence["file"])
    if (keyboard["declaration"] not in (root / keyboard["readme"]["file"]).read_text(encoding="utf-8")
            or keyboard["distributionLicense"] not in keyboard["licenseExpression"].split(" OR ")):
        raise ValueError("Keyboard source license is not authorized upstream")
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
        if source.get("dictionary"):
            # Follow the same imports/aliases as the generator, plus its separate character and English inputs.
            def visit(relative):
                if relative in selected:
                    return
                path = snapshot / relative
                if not path.resolve().is_relative_to(snapshot.resolve()):
                    raise ValueError("Source dictionary escapes its snapshot")
                selected.add(relative)
                _, imports, _ = parse_header(path.read_text(encoding="utf-8").splitlines())
                for name in imports:
                    if name not in source.get("skipImports", []):
                        imported = resolve_import(source.get("importAliases", {}).get(name, name), str(path.parent))
                        visit(Path(imported).relative_to(snapshot).as_posix())
            visit(source["dictionary"])
            selected.update(source.get("extraChars", []))
            selected.update(source.get("english", []))
        for path in sorted(snapshot.rglob("*")):
            if not path.is_file():
                continue
            relative = path.relative_to(snapshot).as_posix()
            notice = relative in {"LICENSE", "LICENSE-SCOPE.md", "NOTICE.md"}
            if not notice and (not relative.endswith(".dict.yaml") or (selected and relative not in selected)):
                continue
            files[source["name"] + "/" + relative] = path
    for name in [*TOOL_INPUTS, "data/hotwords.tsv", "data/blocklist.tsv", "data/tatoeba-freq.tsv",
                 "third_party/manifests/default-data.json",
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
    # The separate source distribution supplies solid XZ compression; stored inner ZIPs
    # preserve familiar rebuild paths without hiding duplicate text from that compressor.
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_STORED) as bundle:
        for name, path in sorted(files.items()):
            entry = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            entry.compress_type = zipfile.ZIP_STORED
            bundle.writestr(entry, path.read_bytes())
        for name, content in {"source-manifest.json": json.dumps(record, ensure_ascii=False, indent=2) + "\n",
                              "README.txt": instructions}.items():
            entry = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            entry.compress_type = zipfile.ZIP_STORED
            bundle.writestr(entry, content.encode("utf-8"))


def collect(output):
    inputs = json.loads((ROOT / "third_party/manifests/notice-inputs.json").read_text(encoding="utf-8"))
    for license_text in inputs["licenseTexts"]:
        if sha256(ROOT / license_text["file"]) != license_text["sha256"]:
            raise ValueError("License text no longer matches its recorded source: " + license_text["file"])
    authorization = check_authorization()
    run(["go", "mod", "verify"], APP)
    components = collect_rust(output)
    # These remain independent licensed data. Include exact editable inputs and the
    # generator, including the complete decomposition upstream archive (GPL source).
    from make_input_tools import check as check_input_tools, prepare_sources
    input_manifest_path = ROOT / "third_party/manifests/input-tools.json"
    input_manifest = json.loads(input_manifest_path.read_text(encoding="utf-8"))
    check_input_tools(ROOT / ".local/data")
    input_sources = prepare_sources(input_manifest)
    for source in input_manifest["sources"]:
        archive = ROOT / ".local/third-party" / ("input-tools-" + source["name"] + "-" + source["revision"] + ".zip")
        archive.parent.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_STORED) as bundle:
            files = {}
            for item in source["files"]:
                files[source["name"] + "/" + item["name"]] = (input_sources / source["name"] / item["name"]).read_bytes()
            files["tools/make_input_tools.py"] = (ROOT / "tools/make_input_tools.py").read_bytes()
            files["third_party/manifests/input-tools.json"] = input_manifest_path.read_bytes()
            files["README.txt"] = ("FreeType independent input lookup data. Original inputs and licenses are retained.\n"
                            "Build with Python 3.11+: python -X utf8 tools/make_input_tools.py\n"
                            "The manifest pins all sources and hashes; the generator downloads any other required inputs.\n").encode("utf-8")
            for name, content in sorted(files.items()):
                bundle.writestr(zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0)), content)
        component = copy_component(output, "data:" + source["name"], source["revision"],
                                   ROOT / "third_party/licenses", [source["name"] + "-LICENSE.txt"], archive)
        component["sourceUrl"] = source["repository"]
        components.append(component)
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
    components.append(copy_component(output, "data:opencc", "1.4.1",
                                     ROOT / "third_party/licenses", ["OpenCC-LICENSE.txt", "OpenCC-AUTHORS.txt"],
                                     ROOT / "core/data/s2t.tsv"))
    licenses = ROOT / "third_party/licenses"
    components.append(copy_component(output, "engine:wowtype-core", authorization["wowtype"]["revision"], licenses,
                                     ["Apache-2.0-LICENSE.txt", "WowType-README.txt", "WowType-Cargo.toml.txt"]))
    keyboard = authorization["wowtypeKeyboard"]
    components.append(copy_component(output, "ui:wowtype-keyboard", keyboard["revision"], licenses,
                                     ["Apache-2.0-LICENSE.txt", "WowType-STATS-README.txt", "WowType-KEYBOARD-NOTICE.txt"],
                                     ROOT / keyboard["source"]["file"]))
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


def export_distribution(staged, output, source_bundle, source_tree=None):
    """Keep notices installed; preserve exact corresponding sources in a separate, identified artifact."""
    manifest = json.loads((staged / "manifest.json").read_text(encoding="utf-8"))
    source_files = {name for component in manifest["components"] for name in component["sourceFiles"]}
    if source_tree:
        # The recorded Git tree includes the exact converters and FTD compiler used for this build.
        project_source = "project/FreeType-source.tar"
        target = staged / project_source
        target.parent.mkdir(parents=True, exist_ok=True)
        run(["git", "archive", "--format=tar", "--mtime=1970-01-01T00:00:00Z",
             "--output=" + str(target.resolve()), source_tree])
        manifest["projectSource"] = {"file": project_source, "tree": source_tree, "sha256": sha256(target)}
        manifest["files"][project_source] = sha256(target)
        source_files.add(project_source)
        (staged / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    for relative, expected in manifest["files"].items():
        path = staged / relative
        if not path.resolve().is_relative_to(staged.resolve()) or sha256(path) != expected:
            raise ValueError("Source/notice snapshot changed: " + relative)
    source_bundle.parent.mkdir(parents=True, exist_ok=True)
    # Normalize metadata so the source artifact is reproducible and contains no local paths or identities.
    with tarfile.open(source_bundle, "w:xz", preset=6) as archive:
        for relative in sorted([*manifest["files"], "manifest.json"]):
            path = staged / relative
            member = tarfile.TarInfo(relative)
            member.size = path.stat().st_size
            member.mode = 0o644
            with path.open("rb") as content:
                archive.addfile(member, content)
    # Verify every member before associating the archive with an installer.
    expected_files = {**manifest["files"], "manifest.json": sha256(staged / "manifest.json")}
    verified = set()
    with tarfile.open(source_bundle, "r|xz") as archive:
        for member in archive:
            with archive.extractfile(member) as content:
                if member.name not in expected_files or hashlib.file_digest(content, "sha256").hexdigest() != expected_files[member.name]:
                    raise ValueError("Source archive verification failed: " + member.name)
            verified.add(member.name)
    if verified != expected_files.keys():
        raise ValueError("Source archive is incomplete")
    for relative in manifest["files"]:
        if relative not in source_files:
            target = output / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(staged / relative, target)
    manifest["format"] = 2
    manifest["files"] = {name: digest for name, digest in manifest["files"].items() if name not in source_files}
    for component in manifest["components"]:
        component["externalSourceFiles"] = component["sourceFiles"]
        component["sourceFiles"] = []
    manifest["sources"] = {"file": source_bundle.name, "sha256": sha256(source_bundle),
                           "bytes": source_bundle.stat().st_size,
                           "distribution": "local-only",
                           "projectRepository": "https://github.com/OUBIGFA/FreeType"}
    instructions = (
        "FreeType 第三方源码与本地重建记录\n\n"
        f"本构建的本地来源归档：{source_bundle.name}\n"
        f"SHA-256：{manifest['sources']['sha256']}\n\n"
        "此归档仅供本地重建与核对；不随安装器分发，不作为公开附件\n"
        "FreeType 公开源码：https://github.com/OUBIGFA/FreeType\n"
        "组件固定版本及上游来源见 manifest.json；GPL、MPL 等对应源码义务仍适用\n"
        "公开分发前须核对准确版本、必要修改和生成脚本的源码获取方式，不能仅用许可文本替代\n"
        "来源包保留原始许可、所需源码、固定数据输入及转换脚本；各组件继续遵循各自许可\n"
        "manifest.json 中的 externalSourceFiles 是来源包内路径；它们不属于运行所需文件\n"
    )
    output.mkdir(parents=True, exist_ok=True)
    (output / "SOURCES.txt").write_text(instructions, encoding="utf-8", newline="\n")
    manifest["files"]["SOURCES.txt"] = sha256(output / "SOURCES.txt")
    (output / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--sources", required=True, type=Path)
    parser.add_argument("--source-tree", required=True)
    arguments = parser.parse_args()
    if arguments.out.exists():
        parser.error("Use a new output directory; existing notices are not overwritten")
    parent = Path(os.environ.get("FREETYPE_TEST_TEMP", ROOT / "_temp/notices"))
    parent.mkdir(parents=True, exist_ok=True)
    staged = Path(tempfile.mkdtemp(prefix="notice-sources-", dir=parent))
    collect(staged)
    manifest = export_distribution(staged, arguments.out, arguments.sources, arguments.source_tree)
    print(f"Collected {len(manifest['components'])} components and {len(manifest['files'])} notice files; "
          f"{len(manifest['unresolved'])} upstream permission issue remains")


if __name__ == "__main__":
    main()
