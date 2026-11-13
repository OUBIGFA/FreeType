# 第三方来源与许可

机器可读记录见 [upstreams.json](manifests/upstreams.json)，仓库检查器会核对其中的修订号和 WowType 归档摘要。

| 来源 | 固定修订 | 用途 | 许可状态 |
| --- | --- | --- | --- |
| WowType | `5656e136e9f5fc32837c87bc1a2242ec159a32d7` | 保留历史；Rust 引擎、C ABI 和行为测试 | 仓库未附许可证文本，`core/Cargo.toml` 声明 `MIT OR Apache-2.0`，待上游确认 |
| mygo | `v0.9.0`（`4811a2869b5a66d2b7e21a9eb5993fcff544460b`） | Windows 托盘、设置窗口和安装器 | MIT |
| msime-windows | `26796f3ed646e2ce75ba39bca6f1352caca9fd67` | 参照词库、快捷短语批量文件格式，以及单按 Shift、成对标点后挪光标的处理思路；本地对照副本在 `.local/references/msime-windows`（不入库） | GPL-3.0；未复制代码或数据 |
| rime-ice | 构建时下载，修订写入安装包内 `data/SOURCE.txt` | 中文词库与英文词表 | GPL-3.0，许可证随数据分发 |
| WowType 词级语言模型 `words.lm` | 构建时下载并校验 SHA-256 | 整句排序 | CC BY-SA 4.0（见 `data/SOURCE.txt`） |
| Rust 依赖（含 `windows` 0.62.2、`serde`） | `Cargo.lock`，清单见 [cargo-lock.json](manifests/cargo-lock.json) | 引擎、服务进程、输入法 DLL | 各包声明的许可证，多为 MIT 或 Apache-2.0 |
| Go 依赖 | `windows/app/go.mod`、`go.sum` | mygo 及其传递依赖、`golang.org/x/sys` | 各模块声明的许可证 |
| NSIS 3.13 | mygo 构建时下载并校验 SHA-256，不入库 | 生成安装器 | zlib/libpng 许可 |

## 待核项

- WowType 许可证文本缺失。正式对外分发前需上游补齐或书面确认。
- `voice/src/wtlocal` 经原作者同意引入，见其 `NOTICE.md`；Windows 版本暂不启用语音。
- rime-ice 为 GPL-3.0。固定测试安装器随附其许可证；正式发布前需复核整体分发许可。

## mygo 版本约定

每次任务只固定一个 mygo 版本，写在 `windows/app/go.mod` 和本页。任务开始时核对上游最新发布版本，需要升级时在任务内一次完成升级、构建和测试，不在任务中途跟随上游。
