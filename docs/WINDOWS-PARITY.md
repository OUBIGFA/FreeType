# Windows 行为与上游对应

本表用于维护 FreeType 的 Windows 移植。核对对象为 WowType 保留基线 `5656e136e9f5fc32837c87bc1a2242ec159a32d7`，以及 2026-11-13 核对的上游 main `882bd335c0e1484c8e49e00a22c47431a5b9feb4`。后者相对基线没有修改 Rust 引擎和会话的行为代码；主要增加 macOS 外壳、设置和配置契约。FreeType 没有把这些平台文件当作 Windows 功能接入。

mygo 固定为 v0.9.0，本次核对时仍为最新发布版。

## 默认状态：全局

在“常规 → 中英文切换 → 默认状态”选择“全局”。选择后沿用当前中英文状态。主动切换后，其他应用、其他输入框和服务重启都沿用最后一次选择。

“全局”启用时隐藏“每个应用分别记住”选项，以免同时指定两种相反的规则。原有中文、英文默认选项仍可使用。它们决定启动或未记忆应用的初始状态，不会因为保存字体等其他设置而重新切换中英文。

最后选择保存在用户数据目录的 `mode.json`，与 `config.json` 分开。设置窗口整份保存配置时不会覆盖运行状态。首次没有记录时使用中文。保存失败时切换请求返回错误，不把未保存的状态报告为成功。

## 按功能核对

“复用”表示直接使用上游核心；“Windows 补充”表示平台接入或本地已有功能，不表示上游提供了相同 Windows 实现。

| 功能 | 上游依据与 Windows 对应 | 验证入口 / 差异 |
| --- | --- | --- |
| 全拼、简拼、分词、整句、纠错 | `core/src/lib.rs`、`lexicon.rs`、`session.rs`；Windows 通过 `host.rs` 配置引擎 | 核心完整测试；保留本地已有的 `yuan` / `yu'an` 兼容修复 |
| 双拼、模糊音、简繁、生僻字设置 | 上游核心对应配置；`windows/server/src/input.rs::session_config` | 核心测试、服务测试和设置页面测试；Windows 默认值属于产品选择 |
| 空格、回车、数字选词、以词定字 | 上游 `Session::key`；Windows 传入相同平台无关键码 | `core/tests/session_keys.rs`；Windows 允许每页 10 个，因此补齐数字 0 选第 10 个，界面显示 0 |
| 翻页、方向键、展开候选 | 上游会话的页、光标和网格规则；Windows 根据 frame 排版 | 会话测试、`expanded_panel_marks_cursor_row`；展开面板只为当前行显示序号 |
| 数字表情、普通表情、组合字符 | 上游 `emoji::keycap` 和候选字符串原样保留 | Windows 改用 DirectWrite / Direct2D；按字素指定表情字体；测量和绘制共用排版结果 |
| 候选窗、点击、隐藏、缩放 | Windows 自有无焦点弹出窗口；上游 macOS 使用系统文字排版 | `ui_render_tests.rs`：真实绘制、三种字体、四档缩放、三种排列；私有桌面真实窗口点击和隐藏 |
| Shift、中英文、CapsLock | 上游外壳负责模式切换；Windows 在 `keys.rs`、`host.rs` 接入 | 左右 Shift、组合键、重复预判、按应用记忆、全局保存；全局切换使旧按键缓存失效 |
| 按键预判 | Windows TSF 特有的“先询问、再执行”接口 | 预判副本不能学习或忘词；真正接受按键后才写学习记录；DLL 也不能提前改变组字状态 |
| 中文标点、引号、自动加空格 | 复用 `core/src/symbols.rs`、`autospace.rs`；对照 fcitx5 外壳顺序 | 服务测试；成对标点和延后挪光标为 Windows 补充，真实应用兼容仍需下表验收 |
| V 模式、数字转换、日期时间 | 复用核心计算器与日期时间候选 | 核心会话测试；Windows 使用系统本地时间 |
| 工具栏、剪贴板、常用语、符号 | 对照 fcitx5 外壳；Windows 使用 `tools.rs` | 面板层级、搜索、数字选择、网格与翻页测试；剪贴板历史仍由用户主动开启 |
| 表情快捷键与最近表情 | 对照上游 `isEmojiKey` 的开 / 关行为 | 再按同一快捷键关闭面板；预判不提前关闭；选中后记录最近表情 |
| 邮箱、网址、小键盘点 | Windows 已有补充功能，非上游会话功能 | `raw.rs` 和服务测试；PgUp / PgDn 与可见翻页箭头共用入口；数字仍是正文 |
| 快捷短语、外部词库、自己加词 | Windows 批量格式适配，再交给原引擎排序 | `msime.rs`、`dictfmt.rs`、`store.rs` 测试；上游 `custom.tsv` 与 Windows 文件格式不混用 |
| 学习、删除、完整备份 | 上游学习引擎；FreeType 已有完整备份接口 | 核心、服务、管道测试；增加反复预判后仅学习一次、预判不删除记录的回归 |
| 设置、提示、托盘与状态栏 | Windows 补充；Go 与 Rust 使用同一配置字段 | 设置页面行为测试、服务状态测试；浅色 / 深色设置页面截图测试 |
| Windows 平台与安装器 | TSF x64 / x86、命名管道、mygo 和 NSIS | DLL 加载测试、管道端到端测试、安装前准备测试；统一入口 `windows/build.ps1` |

上游来源：[WowType 固定基线](https://github.com/Gloridust/WowType/tree/5656e136e9f5fc32837c87bc1a2242ec159a32d7)、[本次核对的 main](https://github.com/Gloridust/WowType/tree/882bd335c0e1484c8e49e00a22c47431a5b9feb4)、[fcitx5 按键外壳](https://github.com/Gloridust/WowType/blob/882bd335c0e1484c8e49e00a22c47431a5b9feb4/fcitx5-addon/src/wowtype_engine.cpp)、[macOS 候选窗](https://github.com/Gloridust/WowType/blob/882bd335c0e1484c8e49e00a22c47431a5b9feb4/macos/Sources/App/CandidatePanel.swift)。

## 仍需实际安装后验收的范围

- 在记事本、浏览器、Office、商店应用、管理员进程和 32 位应用中实际输入；包括焦点切换、应用返回的光标位置、成对标点挪光标和多显示器 DPI 切换。
- 安装、升级、卸载、管理员授权、TSF 注册、系统输入法列表与开机启动。这些操作不由普通构建自动执行。
- 系统没有安装相应字体的生僻字和较新表情，仍受 Windows 字体覆盖范围限制。

语音和互译尚未接入 Windows 外壳。上游 Linux / macOS 实现继续作为参考，不把它们计入 Windows 已支持或已验收功能。自动测试和私有桌面窗口测试不能代替上述真实应用验收。
