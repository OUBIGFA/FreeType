# FreeType

基于 [WowType](https://github.com/Gloridust/WowType) 准备移植的 Windows 输入法，前端采用 [mygo](https://github.com/egoist/mygo)。

**当前阶段：仓库准备。尚未实现可运行的 Windows 输入法，也没有 Windows 安装包。**

## 当前内容

- `core/`：Rust 拼音引擎、输入会话、C ABI 和原有测试。
- `voice/`、`voice-daemon/`：上游语音代码；Windows 适配和授权核对尚未完成。
- `fcitx5-addon/`、`macos/`：上游平台集成，用于对照输入行为，不代表 FreeType 的平台支持承诺。
- `scripts/`、`tools/`、`data/`、`icons/`、`themes/`：输入法相关脚本、公开输入与程序资源。
- `docs/`：[输入法技术文档](docs/README.md)，重点是 [C ABI](docs/ABI.md) 和 [移植边界](docs/PORTING.md)。

保留 WowType 的提交历史。原始分析记录、截图、过时交接报告和 Telegram 机器人已从当前树移除，不另存本地副本。

mygo 目前只登记参考修订和许可证，未加入 Go 应用或依赖锁文件。Windows 输入法接入、Rust/Go 调用边界和前端实现留待后续阶段。

## 仓库检查

需要 Git 和 Python 3.11，无需安装 Python 第三方依赖。先暂存准备提交的文件，再在仓库根目录运行：

~~~powershell
python -X utf8 tools/check_repo.py
python -X utf8 -m unittest discover -s tests/repository -v
~~~

检查器读取 Git 暂存索引，核对本地边界、凭据、机器路径、Windows 文件名、编码、Markdown 链接、二进制登记和上游来源。测试使用真实临时 Git 仓库，只写入被忽略的 `_temp/`。

CI 在 Windows 和 Linux 上运行同一组检查。它只有仓库读取权限，不构建或发布应用，不接收发布密钥，也不上传安装包。

## 固定测试安装器

首个可用 Windows 实现建立后，代码任务按以下顺序交付：必要测试与构建通过 → 更新固定安装器及校验记录 → 核对源码与安装器快照 → 提交并普通推送 `origin/main`。

| 项目 | 固定约定 |
| --- | --- |
| 测试版本 | `0.0.1` |
| 安装标识 | `io.github.oubigfa.freetype` |
| 安装器 | `output/release/FreeType-0.0.1-Setup.exe` |
| SHA-256 | `output/release/FreeType-0.0.1-Setup.exe.sha256` |
| 构建记录 | `output/release/build-info.json` |

用内部构建号、源码快照和 SHA-256 区分测试包。包名、安装标识和用户数据目录保持稳定，方便覆盖安装测试。安装包及记录不进入 Git；普通推送不授权打标签、公开 Release 或外部上传安装器。

本轮没有应用构建入口，不生成空安装器或虚假校验文件。仓库卫生通过不代表 Windows 构建、输入法功能或桌面验收已通过。

## 来源与许可

- WowType 基线：`1e047f7469f4e2f3da7a8a0cd49dbc8c0b46a0f1`，保留历史，不推送上游标签。
- mygo 固定版本：`v0.9.0`（`4811a2869b5a66d2b7e21a9eb5993fcff544460b`）。
- 仓库组织和检查器参考 [Gitowl](https://github.com/OUBIGFA/Gitowl)，不修改该项目。

完整记录见 [第三方来源与许可](third_party/README.md) 和 [仓库边界](references/repository-boundaries.md)。上游许可正文、语音实现授权范围以及依赖/模型发布审查存在待核项；登记不表示它们已获准进入 FreeType 发布包。

