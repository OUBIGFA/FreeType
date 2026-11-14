# FreeType 第三方声明

本文件随本地测试安装器安装为 `THIRD-PARTY-NOTICES.txt`。完整许可原文和需要提供的源码位于安装目录的 `third-party/`；默认词库的原有声明也保留在 `data/`。

**这份声明不为 FreeType 整包授予许可证，也不表示已经取得所有上游授权。WowType 缺少许可证文本或书面许可，公开分发仍受阻。** 补齐声明、测试通过或标为预发布，都不能代替该授权。

## 1. 如何核对本包

- `third-party/manifest.json` 列出本次构建的组件、版本、许可文件、源码文件、逐文件 SHA-256 和已知未解决项。
- Rust 范围为 x64 服务及 x64/x86 输入法 DLL 的普通依赖图；不把仅用于构建或测试的依赖宣称为随包运行时。
- Go 范围为 Windows 应用实际导入的外部模块，以及 Go 标准库和标准库内附的依赖声明。
- 一个模块的原始声明可能涉及该模块其他平台或附属部分。保留这些原文不表示其中每一段代码都被链接进 Windows 程序；清单不是逐符号链接分析。
- 项目不改写上游许可正文。模块的源码下载包、Rust/Go 版本和数据来源均与构建输入对应。具体义务以上游许可原文为准，不以本摘要代替。
- 本地 `build-info.json` 记录声明清单摘要、五个 Windows 文件的版本资源和实际构建来源；它不作为公开安装包的附件。

## 2. 引擎与应用依赖

### WowType

来源：`https://github.com/Gloridust/WowType`，固定修订 `5656e136e9f5fc32837c87bc1a2242ec159a32d7`。

FreeType 复用其 Rust 引擎、C ABI 和行为测试，并增加 Windows 服务、TSF 输入法和设置程序。原有内部 crate 名与 `wowtype_` ABI 前缀为保持上游兼容而保留。

截至 2026-11-14，固定修订及核对时的上游均未提供根目录许可证。`core/Cargo.toml` 的 `MIT OR Apache-2.0` 字段不是完整许可文本，不足以解除这个发布阻塞。需取得上游许可证或明确的书面授权后再复核分发范围；本项目不代上游补写版权授权。

### Rust 与 Go 模块

- Rust 包由 `Cargo.lock` 固定；许可原文位于 `third-party/rust/<包名>@<版本>/`。包含 `windows` 系列绑定、Serde 和 Unicode 相关组件等。MIT、Apache、Unicode 等声明分别保留，不统一改写成一种许可。
- mygo 固定为 `v0.9.0`，来源 `https://github.com/egoist/mygo`，采用 MIT 许可。应用和运行时的许可及附属声明位于 `third-party/go/github.com/egoist/mygo@v0.9.0/`。
- 更新功能使用 `github.com/creativeprojects/go-selfupdate v1.7.1`。它和其余实际导入的 Go 模块按 `windows/app/go.mod`、`go.sum` 固定，原文位于 `third-party/go/<模块>@<版本>/`，完整列表见随包清单。
- `github.com/hashicorp/go-cleanhttp v0.5.2`、`go-retryablehttp v0.7.8`、`go-version v1.9.0` 使用 MPL-2.0。FreeType 未修改这些模块；每个模块目录的 `source/` 内提供对应的原始 Go 模块 zip，可离线取得其源码。修改这些模块时必须同步更新对应源码及声明。
- Go 运行时使用 Go `1.27.4`，保留 BSD 许可、PATENTS 和标准库内附依赖声明，见 `third-party/toolchain/go@go1.27.4/`。
- Rust 标准库声明保留在 `third-party/toolchain/rust@<实际版本>/COPYRIGHT-library.html`，版本由构建时的工具链确定。它不表示 FreeType 分发了整个 Rust 编译器。

### 公共后缀数据

`golang.org/x/net v0.62.0` 的 `publicsuffix` 表使用 Public Suffix List，来源 `https://publicsuffix.org/`，采用 MPL-2.0。

固定数据修订为 `d6c92f1bbb7433e5db7b8405c25d4035fb8ff376`，仓库为 `https://github.com/publicsuffix/list`。`third-party/data/public-suffix-list@<修订>/` 包含 MPL 原文和 `source/` 中的原始 `public_suffix_list.dat`。FreeType 不修改该数据。构建会核对模块版本及其表中记录的源修订；升级依赖时不能沿用旧数据的声明。

## 3. 默认数据与图标

- 白霜主库来自 `gaboolic/rime-frost` 修订 `555a736932de245d7f59f640a98bd58f368792b3`，采用 GPL-3.0。其上游词表来源需与固定输入一并核对。
- 万象补充来自 `amzxyz/rime-wanxiang-data` 修订 `4b50697bc3fa45ea7f79faebcc731fb42a3b6d8c`。作者为 amzxyz，其原创字词数据采用 CC BY 4.0；此范围不覆盖其程序代码及第三方组件。只合入字表、基础词、兼容读音、地名、名人和艺人六张核心表，不复制万象程序或 `.gram` 模型。
- 数据转换包括去除拼音声调、将 ü 转为 v、按引擎量级归一化词频、白霜优先去重、万象补缺，以及合入项目热词。原始来源、转换输入摘要和改动说明见 `data/SOURCE.txt`、`data/MANIFEST.json`；生成脚本及固定输入配置位于 FreeType 源码仓库的 `tools/make_default_data.py` 和 `third_party/manifests/default-data.json`。
- 词级语言模型 `words.lm` 来自 WowType 的 `lm-2026.11.09` 数据发布，采用 CC BY-SA 4.0。本包未修改模型内容；下载来源和摘要见默认数据清单。该数据许可不补足 WowType 程序代码的授权。
- GPL-3.0、CC BY 4.0、CC BY-SA 4.0 原文在 `data/LICENSE`；万象的原始许可范围和第三方声明在 `data/WANXIANG-LICENSE-SCOPE.md`、`data/WANXIANG-NOTICE.md`。`third-party/data/default@<数据版本>/` 另保留这些声明。公开分发前仍须复核 GPL 数据的对应源码及转换输入提供方式和组合分发义务，不能只保留一个许可证名称。
- `data/s2t.tsv` 是由 WowType 固定修订带入的 OpenCC 字符级简繁映射，不含 OpenCC 程序。本包保留映射内容；上游转换脚本为 `tools/opencc2tsv.py`。历史生成时的 OpenCC 包版本未在上游记录，因此不冒称来自某个 OpenCC 发行版。OpenCC 项目为 `https://github.com/BYVoid/OpenCC`，采用 Apache-2.0；完整许可和作者名单在 `third-party/data/opencc@wowtype-5656e136/`，转换后的表也在其 `source/` 内提供。
- 关于页的 GitHub 标志来自 `primer/octicons` 修订 `68ec2de8d0ddfcbb7497da01d0b7d98eb5593020`，采用 MIT 许可。FreeType 只调整 SVG 填充以跟随文字颜色，许可原文在 `third-party/asset/octicons@<修订>/`。

## 4. NSIS 安装器

mygo 使用 NSIS `3.13` 及 Modern UI 生成安装器，并使用 LZMA 压缩。来源为 `https://nsis.sourceforge.io/`。

NSIS `COPYING` 区分通用代码的 zlib/libpng 许可、bzip2 压缩模块的 bzip2 许可，以及 LZMA 压缩模块的 CPL-1.0。原文还包含 LZMA 的链接例外；不能把所有部分统一写成 zlib，也不能把该例外当成免除对 LZMA 自身修改的义务。

`third-party/installer/nsis@3.13/` 保留 `COPYING`、Modern UI 许可，以及 `source/nsis-3.13-src.tar.bz2` 对应源码。FreeType 未修改 NSIS 源码；安装器行为由项目的 `windows/app/installer.nsh` 与 mygo 模板生成。源码归档摘要是项目固定输入，不是发布方数字签名。构建会检查实际 NSIS 版本与所附源码版本相同。

## 5. 更新这份声明

升级依赖、替换图标、改动词库或修改上游代码后，重新运行统一构建入口。构建从锁定依赖提取原文；缺许可文件、缺 MPL 对应源码、数据修订不匹配或摘要不符时停止，不能复用另一份安装器的声明。

项目来源登记为 `https://github.com/OUBIGFA/FreeType/tree/main/third_party`。公开发布前还必须解决本文件列出的授权和分发条件，并完成隔离环境安装验收。
