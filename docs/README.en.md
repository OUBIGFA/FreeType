# Input method documentation

[简体中文](README.md) | English

These documents describe the WowType core, the C ABI, and the FreeType Windows shell. The Linux and macOS installation steps and feature states are technical references. They are not FreeType platform commitments.

- [Windows version](WINDOWS.md): Windows structure, input behavior, dictionary import, data paths, build, and verification scope.
- [C ABI](ABI.md): the interface between platform shells and the Rust core.
- [Porting boundaries](PORTING.md): responsibilities of the input session and platform integration.
- [Engine](ENGINE.md): pinyin, candidates, and ranking.
- [Data sources](DATA.md): dictionaries, models, and data formats.
- [Voice](VOICE.md): the upstream voice implementation and its limits.
- [Known limits](LIMITATIONS.md): limits recorded by the upstream implementation.
- [Usage](USAGE.md) and [Configuration](CONFIG.md): core feature and configuration references.
- [Upstream installation](INSTALL.md): retained Linux build information. It is not the Windows installation guide.

See the [project README](../README.en.md) for the product overview and test build rules. See [Third-party sources and licenses](../third_party/README.md) for sources, revisions, and license status.

Most technical pages remain in Chinese. The English index points to the same source documents so that file names and implementation boundaries stay consistent.
