"""Collect notices from the locked Windows dependencies and verified inputs."""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile

from make_default_data import checked_download, sha256


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
    if not mpl_source and not any(re.search(r"license|licence|copying|copyright", path.name, re.I) for path in paths):
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


def collect(output):
    inputs = json.loads((ROOT / "third_party/manifests/notice-inputs.json").read_text(encoding="utf-8"))
    for license_text in inputs["licenseTexts"]:
        if sha256(ROOT / license_text["file"]) != license_text["sha256"]:
            raise ValueError("License text no longer matches its recorded source: " + license_text["file"])
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
    data = ROOT / ".local/data"
    components.append(copy_component(output, "data:default", (data / "VERSION").read_text(encoding="utf-8").strip(),
                                     data, ["LICENSE", "SOURCE.txt", "WANXIANG-LICENSE-SCOPE.md", "WANXIANG-NOTICE.md"]))
    manifest = {
        "format": 1,
        "scope": "Windows normal Rust dependency graphs, imported Go modules, bundled assets and runtime notices; not a symbol-level linkage inventory",
        "components": sorted(components, key=lambda item: (item["id"], item["version"])),
        "unresolved": [{"name": "WowType", "revision": "5656e136e9f5fc32837c87bc1a2242ec159a32d7",
                        "reason": "Missing upstream license text or written permission; public distribution remains blocked"}],
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
