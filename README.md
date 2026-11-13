<div align="center">

<img src="dist/FreeType_logo.png" width="160" alt="FreeType 图标">

# FreeType

可学习、支持自定义词库的 Windows 中文输入法

[English](README.en.md) | 简体中文

![Platform](https://img.shields.io/badge/platform-Windows%2010%2F11%20x64-555555) ![Stage](https://img.shields.io/badge/stage-test%20version-555555)

</div>

## FreeType 是什么

FreeType 是一个 Windows 中文输入法。它适合日常聊天、写作、编程和办公。

它支持全拼、双拼、模糊音和错字纠正。它会记住你的选词习惯。你也可以导入自己的词库和快捷短语。

FreeType 的输入引擎在本机运行。普通中文输入不需要登录账号，也不需要上传正在输入的文字。

## 为什么使用 FreeType

- **越用越顺手**：它会根据你的选词和常用词搭配调整候选顺序。
- **打错也能继续**：按错相邻键、两个键顺序反了，也能找到正确候选。
- **词库容易迁移**：可以导入水杉、搜狗、Rime、WowType 等格式的词库和短语。
- **输入方式灵活**：支持全拼、七种双拼方案、模糊音和多种翻页键。
- **中文排版更自然**：支持中文标点、成对标点和中英文自动加空格。
- **常用内容随时可用**：快捷短语、符号、emoji、日期时间和计算器都可以在输入时使用。
- **设置清楚**：候选窗的字体、字号、方向、数量和颜色都可以调整。
- **数据在自己的电脑上**：词库、学习记录和设置保存在本机。

## 主要功能

### 更适合你的拼音输入

- 全拼输入。
- 双拼输入：小鹤、自然码、微软、搜狗、智能 ABC、紫光和拼音加加。
- 模糊音设置，例如把 `z` 和 `zh` 视为相近读音。
- 错字纠正，例如 `weixni` 可以找到“微信”。
- 整句候选和词语候选一起显示。
- 词库和学习记录会影响候选顺序。
- 可以显示更多生僻字。

### 中文、英文和标点切换

- 单按 Shift 切换中文和英文。
- 可以使用左右 Shift，也可以只使用其中一个。
- 可以让所有应用共用状态，也可以让每个应用记住自己的状态。
- 中文状态下自动使用中文标点。
- 数字后面的点保持半角，例如 `3.14`。
- 输入 `（`、`“`、`《`、`【` 或 `「` 时，可以自动补上右半边。
- 中文和英文、数字之间可以自动加空格。

### 工具栏和快捷输入

在全拼模式下输入 `v`，再按空格，可以打开工具栏。工具栏包含：

- 剪贴板历史
- 快捷短语
- 符号
- emoji

还可以使用以下功能：

- 输入 `riqi`、`shijian`、`xingqi`、`rq` 或 `sj`，查日期和时间。
- 输入 `v1+2*3`，计算算式。
- 输入 `v1314`，显示中文数字和大写金额。
- 输入 `@`，补全常用邮箱后缀。
- 输入网站名后按句号，补全 `.com`、`.cn` 等域名后缀。

### 候选窗可以自己调整

你可以在设置中调整：

- 候选窗字体。
- 字号。
- 横排或竖排。
- 每页显示的候选数量。
- 选中候选的颜色和样式。
- 候选窗提示的位置。

### 词库和快捷短语

FreeType 支持导入：

- 水杉词库和快捷短语。
- 搜狗细胞词库（`.scel`）。
- 搜狗文本词库。
- Rime 词库和快捷短语。
- WowType 词库。
- 只有词语的文本列表。
- 搜狗自定义短语。
- 微信输入法常用语导出文件。

导入的词库会参与组词和排序。它不会覆盖你的学习记录。

你也可以在设置中手动添加词语。拼音可以留空，FreeType 会尝试自动填写常用读音。

### 学习记录可以备份

设置中的“学到的词”页面可以：

- 搜索学到的词。
- 删除某个词的记忆。
- 添加自己的词语。
- 清空学习记录。
- 导出完整学习备份。
- 从备份恢复词语和输入习惯。

完整备份包含词语、选词次数、最近使用时间和词语搭配。请把备份放在安全的位置。

## 设置页预览

下面的图片来自本机安装版 FreeType。常规页放在第一张。

<table>
<tr>
<td><img src="windows/assets/readme/settings-general-installed.png" width="420" alt="FreeType 常规设置页"><br>常规</td>
<td><img src="windows/assets/readme/settings-typing-installed.png" width="420" alt="FreeType 拼音与按键设置页"><br>拼音与按键</td>
</tr>
<tr>
<td><img src="windows/assets/readme/settings-features-installed.png" width="420" alt="FreeType 实用功能设置页"><br>实用功能</td>
<td><img src="windows/assets/readme/settings-dicts-installed.png" width="420" alt="FreeType 词库设置页"><br>词库</td>
</tr>
</table>

### 候选窗颜色

候选窗支持四种选中样式：

<table>
<tr>
<td><img src="windows/assets/readme/candidate-pink.png" width="260" alt="粉色候选窗"><br>粉色（默认）</td>
<td><img src="windows/assets/readme/candidate-blue.png" width="260" alt="蓝色候选窗"><br>蓝色</td>
</tr>
<tr>
<td><img src="windows/assets/readme/candidate-green.png" width="260" alt="微信绿候选窗"><br>微信绿</td>
<td><img src="windows/assets/readme/candidate-sogou.png" width="260" alt="搜狗样式候选窗"><br>搜狗样式（无底色块）</td>
</tr>
</table>

## 快速开始

1. 安装 FreeType。
2. 在 Windows 输入法列表中启用 FreeType。
3. 切换到 FreeType，输入 `nihao`。
4. 按空格或数字键选择候选。
5. 打开“FreeType 设置”，根据自己的习惯调整输入法。

### 常用按键

| 按键 | 作用 |
| --- | --- |
| 单按 `Shift` | 切换中文和英文 |
| 空格 | 选择当前候选 |
| `1` 到 `9` | 选择对应候选 |
| 回车 | 原样上屏正在输入的字母 |
| `-`、`=`、`PgUp`、`PgDn` | 翻页 |
| `←`、`→`、`↓` | 移动候选；横排时 `↓` 可以展开候选面板 |
| `[`、`]` | 上屏当前候选的第一个字或最后一个字 |
| `Ctrl+Delete` | 删除当前词的学习记忆 |
| `Esc` | 清空正在输入的内容 |
| `v`、空格 | 打开工具栏 |

## 数据和隐私

- 普通中文输入在本机完成。
- 词库、设置、学习记录和快捷短语保存在本机。
- 用户数据目录是 `%APPDATA%\FreeType`。
- 日志目录是 `%LOCALAPPDATA%\FreeType\logs`。
- 剪贴板历史默认关闭。
- 打开剪贴板历史后，最多保留最近 20 条，并保存在内存中。
- 学习备份包含个人输入习惯。请不要公开分享。

## Windows 版本

FreeType 当前面向 Windows 10/11 x64。它是测试版本。不同应用对输入法、候选窗位置和高 DPI 显示的支持可能不同。

如果某个应用已经打开，安装或更新输入法后，请重新打开这个应用。

## 常见问题

### 只能输入英文字母

1. 单按 Shift，切回中文状态。
2. 检查任务栏的输入法按钮是否显示“中”。
3. 打开 FreeType 设置，检查输入法服务是否正在运行。

### 候选窗看不清

打开“FreeType 设置 → 候选窗”，更换字体、字号、排列方向或选中样式。

### 想让某个词排在前面

多次选择这个词。FreeType 会学习你的选择。也可以在“学到的词”中手动添加词语。

### 想使用自己的词库

打开“FreeType 设置 → 词库”，选择词库文件。导入搜狗、Rime 或其他支持的格式时，FreeType 会显示导入结果。

### 想保存输入习惯

打开“FreeType 设置 → 学到的词”，点击“导出”。换电脑或重新安装后，再点击“导入”。

## 相关文档

- [Windows 版详细说明](docs/WINDOWS.md)
- [配置说明](docs/CONFIG.md)
- [输入法技术文档](docs/README.md)

FreeType 基于 [WowType](https://github.com/Gloridust/WowType)。Windows 设置窗口和托盘程序使用 [mygo](https://github.com/egoist/mygo)。
