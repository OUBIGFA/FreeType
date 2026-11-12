# FreeType

基于 [WowType](https://github.com/Gloridust/WowType) 的 Windows 中文输入法，托盘和设置界面采用 [mygo](https://github.com/egoist/mygo)，支持导入 [水杉输入法](https://github.com/metasequoiaime/msime-windows) 格式的词库和快捷短语。

**当前阶段：测试版 0.0.1。** 自动测试已覆盖引擎、服务进程、命名管道、输入法 DLL 加载和设置界面；在真实应用里打字的桌面验收尚未完成。结构、构建和验证范围见 [Windows 版说明](docs/WINDOWS.md)。

## 当前内容

- `windows/`：Windows 版。`tsf/` 输入法 DLL，`server/` 服务进程，`protocol/` 管道消息，`app/` mygo 托盘与设置程序，`build.ps1` 构建固定测试安装器。
- `core/`：Rust 拼音引擎、输入会话、C ABI 和原有测试；`core/src/msime.rs` 读写水杉格式。
- `voice/`、`voice-daemon/`：上游语音代码；Windows 适配和授权核对尚未完成。
- `fcitx5-addon/`、`macos/`：上游平台集成，用于对照输入行为，不代表 FreeType 的平台支持承诺。
- `scripts/`、`tools/`、`data/`、`icons/`、`themes/`：输入法相关脚本、公开输入与程序资源。
- `docs/`：[输入法技术文档](docs/README.md)，重点是 [C ABI](docs/ABI.md) 和 [移植边界](docs/PORTING.md)。

保留 WowType 的提交历史。原始分析记录、截图、过时交接报告和 Telegram 机器人已从当前树移除，不另存本地副本。

面向用户的名称统一为 FreeType。为便于和上游同步，内部 crate 名、C ABI 前缀 `wowtype_` 和上游 Linux/macOS 外壳保持原名。

## 仓库检查

需要 Git 和 Python 3.11，无需安装 Python 第三方依赖。先暂存准备提交的文件，再在仓库根目录运行：

~~~powershell
python -X utf8 tools/check_repo.py
python -X utf8 -m unittest discover -s tests/repository -v
~~~

检查器读取 Git 暂存索引，核对本地边界、凭据、机器路径、Windows 文件名、编码、Markdown 链接、二进制登记和上游来源。测试使用真实临时 Git 仓库，只写入被忽略的 `_temp/`。

CI 在 Windows 和 Linux 上运行同一组检查。它只有仓库读取权限，不构建或发布应用，不接收发布密钥，也不上传安装包。Windows 应用的测试与构建入口见 [Windows 版说明](docs/WINDOWS.md#构建)。

## 固定测试安装器

代码任务按以下顺序交付：必要测试与构建通过 → 运行 `windows/build.ps1` 更新固定安装器及校验记录 → 核对源码与安装器快照 → 提交并普通推送 `origin/main`。

| 项目 | 固定约定 |
| --- | --- |
| 测试版本 | `0.0.1` |
| 安装标识 | `io.github.oubigfa.freetype` |
| 安装器 | `output/release/FreeType-0.0.1-Setup.exe` |
| SHA-256 | `output/release/FreeType-0.0.1-Setup.exe.sha256` |
| 构建记录 | `output/release/build-info.json` |

用内部构建号、源码快照和 SHA-256 区分测试包。包名、安装标识和用户数据目录保持稳定，方便覆盖安装测试。安装包及记录不进入 Git；普通推送不授权打标签、公开 Release 或外部上传安装器。

`build-info.json` 的 `source.tree` 是构建时工作目录的 Git 树对象，提交后应等于该提交的树。构建不安装、不注册输入法；仓库卫生和自动测试通过不代表桌面验收已通过。

## 来源与许可

- WowType 基线：`1e047f7469f4e2f3da7a8a0cd49dbc8c0b46a0f1`，保留历史，不推送上游标签。
- mygo 固定版本：`v0.9.0`（`4811a2869b5a66d2b7e21a9eb5993fcff544460b`）。
- 仓库组织和检查器参考 [Gitowl](https://github.com/OUBIGFA/Gitowl)，不修改该项目。

完整记录见 [第三方来源与许可](third_party/README.md) 和 [仓库边界](references/repository-boundaries.md)。上游许可正文、语音实现授权范围以及依赖/模型发布审查存在待核项；登记不表示它们已获准进入 FreeType 发布包。

