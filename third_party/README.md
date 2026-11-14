# 第三方来源与许可

机器可读记录见 [upstreams.json](manifests/upstreams.json)，仓库检查器会核对其中的修订号和 WowType 归档摘要。
默认词库与语言模型的固定输入见 [default-data.json](manifests/default-data.json)。数据生成和打包会核对归档、转换输入及输出摘要。

随包总声明见 [NOTICE.md](NOTICE.md)。统一构建入口将它安装为 `THIRD-PARTY-NOTICES.txt`，并从实际 Rust/Go 依赖归集许可原文及必要源码到 `third-party/`。生成清单记录组件版本和逐文件 SHA-256，不入库；固定许可文本、公共后缀数据与 NSIS 源码输入见 [notice-inputs.json](manifests/notice-inputs.json)。

| 来源 | 固定修订 | 用途 | 许可状态 |
| --- | --- | --- | --- |
| WowType | `5656e136e9f5fc32837c87bc1a2242ec159a32d7` | 保留历史；Rust 引擎、C ABI 和行为测试 | README 明确 MIT 或 Apache-2.0 任选；本次选择 Apache-2.0，原文和依据见 [authorization.json](manifests/authorization.json) |
| mygo | `v0.9.0`（`4811a2869b5a66d2b7e21a9eb5993fcff544460b`） | Windows 托盘、设置窗口和安装器 | MIT |
| msime-windows | `26796f3ed646e2ce75ba39bca6f1352caca9fd67` | 参照词库、快捷短语批量文件格式，以及单按 Shift、成对标点后挪光标的处理思路；本地对照副本在 `.local/references/msime-windows`（不入库） | GPL-3.0；未复制代码或数据 |
| rime-frost（白霜） | `555a736932de245d7f59f640a98bd58f368792b3` | 默认中文主库、细胞词库、字表与英文词表；由雾凇发展而来，不另叠加雾凇整库 | GPL-3.0，许可证随数据分发 |
| rime-wanxiang-data（万象） | `4b50697bc3fa45ea7f79faebcc731fb42a3b6d8c` | 默认核心词库补缺；只使用字词数据，不复制程序代码或语法模型 | 原创数据为 CC BY 4.0；保留作者署名、来源、改动说明及许可范围 |
| WowType 词级语言模型 `words.lm` | 构建时下载并校验 SHA-256 | 整句排序 | CC BY-SA 4.0（见 `data/SOURCE.txt`） |
| Rust 依赖（含 `windows` 0.62.2、`serde`） | `Cargo.lock`，清单见 [cargo-lock.json](manifests/cargo-lock.json) | 引擎、服务进程、输入法 DLL | 各包声明的许可证，多为 MIT 或 Apache-2.0 |
| Go 依赖 | `windows/app/go.mod`、`go.sum` | mygo 及其传递依赖、`golang.org/x/sys` | 各模块声明的许可证 |
| Go 工具链 | `1.27.4` | Windows 设置程序与构建工具；mygo 的最低要求仍为 1.27.3 | BSD-3-Clause |
| OpenCC 字符与词组映射 | `1.4.1`；生成表固定在 `core/data/s2t.tsv` | 字符与词组级简繁转换，不包含 OpenCC 程序 | Apache-2.0；随包保留许可、作者名单及转换后的表 |
| Unicode CLDR emoji | WowType 固定修订的 `core/src/emoji/cldr.tsv` | 编入引擎的中英文 emoji 关键词 | Unicode-3.0；保留 Unicode 版权声明、原始数据及许可全文，不分发 LDML 规范文档 |
| 公共后缀数据 | `d6c92f1bbb7433e5db7b8405c25d4035fb8ff376`，对应 `golang.org/x/net v0.62.0` | 域名公共后缀判断 | MPL-2.0；随包保留原始数据和许可 |
| NSIS 3.13 | mygo 固定二进制下载摘要；项目另固定源码归档摘要，不入库 | 生成安装器 | 通用代码 zlib/libpng、bzip2 模块 bzip2、LZMA 模块 CPL-1.0（含链接例外）；保留完整 COPYING 与源码 |

## 分发核对

- 固定修订的 README 与 Cargo 声明共同记录 WowType 的双许可；根目录缺少 LICENSE 不再作为“没有公开授权”的依据。本次选择 Apache-2.0，保留标准全文、原文证据、修改标识和原有署名。构建校验依据摘要，改动时必须重新核对。
- 构建归集选定 Windows Rust 依赖图及实际导入的 Go 模块原文，不以许可名称代替正文。Go 的三个 Hashicorp MPL 模块附原始模块 zip，NSIS 附对应源码，公共后缀数据附原始文件；改动或升级后需重新核对。
- `voice/src/wtlocal` 经原作者同意引入，见其 `NOTICE.md`；该上游语音组件不属于本次 Windows 安装包。
- 白霜为 GPL-3.0。随包 `default-data-sources.zip` 提供含上游雾凇等来源头部的原始词表、转换工具、配置和热词，保留派生词表的 GPL 分发条件，不仅提供许可证名称。可用 `--snapshot-root` 从这些材料重建。
- 未修改的词级模型单独按 CC BY-SA 4.0 分发，保留 WowType contributors、LCCC 和中文维基百科署名及发布说明。程序、模型和数据分别登记，不宣称整包统一采用某一种许可。
- 当前 OpenCC 映射由 `tools/opencc2tsv.py` 按 1.4.1 的字符与完整词组数据生成；原始词组输入和生成步骤见脚本说明。生成表、许可及作者名单随包提供，不分发 OpenCC 程序。
- 万象的 CC BY 4.0 适用于其原创字词数据，不覆盖处理脚本或第三方组件。仅合入六张默认核心数据表；随包保留 `WANXIANG-LICENSE-SCOPE.md`、`WANXIANG-NOTICE.md`，并在 `SOURCE.txt` 标明 amzxyz 署名及转换改动。

## mygo 版本约定

每次任务只固定一个 mygo 版本，写在 `windows/app/go.mod` 和本页。任务开始时核对上游最新发布版本，需要升级时在任务内一次完成升级、构建和测试，不在任务中途跟随上游。
