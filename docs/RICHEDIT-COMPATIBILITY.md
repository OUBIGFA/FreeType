# RichEdit 输入兼容性

2026-11-26 核查 WinOne 序号说明框“字母直接输入、没有中文候选”的问题。

## 已确认的问题

FreeType 把两种不同状态合并在 `State::sensitive` 中：Windows 临时禁止输入法，以及输入框声明为密码或 PIN。`check_sensitive` 只有取得只读编辑会话后才清除这个状态。

当非编辑区域先报告 `EMPTYCONTEXT` 或 `KEYBOARD_DISABLED`，后来输入框允许输入，但没有授予只读锁时，旧状态一直保留。`OnTestKeyDown` 因而直接放行字母，不询问转换引擎，不显示中文候选。私有桌面的真实 Windows TSF 管理器加可控文本存储已复现：修复前第一个字母被放行；修复后逐键输入 `ceshi`，空格上屏为“测试”。

修复按当前上下文读取禁用标记，不再让临时禁用状态依赖读取正文成功才解除。已确认的密码/PIN 范围单独绑定其上下文；同一密码框暂时不给读锁时继续保护，其他输入框不继承这个状态。同时按微软规定用 `GetAppProperty(GUID_PROP_INPUTSCOPE)` 读取应用声明的范围。

## 与成熟方案的差异

| 环节 | 修复前 FreeType | 水杉 Windows | 本次处理 |
| --- | --- | --- | --- |
| 禁用输入法 | 与密码状态合并缓存，读锁失败时沿用旧值 | `_IsKeyboardDisabled` 每次查询当前文档的禁用隔间，局部值从 false 开始 | 复用按当前上下文判断的策略，保留 FreeType 自身密码保护 |
| 密码/PIN 范围 | 调用 `GetProperty` | 所查正式版和开发版也调用 `GetProperty` | 按微软 `ITfInputScope` 文档改为 `GetAppProperty`，由真实宿主属性测试核对 |
| 写入会话 | 按键写入要求同步 | `_InvokeKeyHandler` 使用 `TF_ES_ASYNCDONTCARE`，与微软 SampleIME 一致 | 没有证据表明此差异导致本次“不出候选”，本次保留既有写入时序 |

对照固定修订：水杉正式版 `v0.14.1` / `c9fae22421c124787a35aec65f0c80b47a7195af`；开发分支 `9b0f568937ce7edbea3a4fc98645435d68e67f74`。开发分支相比上次记录增加 7 次提交，相关 `KeyEventSink.cpp`、`KeyHandler.cpp` 未变化；复用上次固定快照 `d8e9ebaabac5f7f63ba01c70fec39a6111e3b13d` 的对应代码证据。本次参考行为与 Windows 接口契约，未复制水杉程序代码。

来源：[水杉按键处理](https://github.com/metasequoiaime/msime-windows/blob/9b0f568937ce7edbea3a4fc98645435d68e67f74/windows/src/Key/KeyEventSink.cpp)、[水杉写入会话](https://github.com/metasequoiaime/msime-windows/blob/9b0f568937ce7edbea3a4fc98645435d68e67f74/windows/src/Key/KeyHandler.cpp)、[微软 InputScope](https://learn.microsoft.com/en-us/windows/win32/api/inputscope/nn-inputscope-itfinputscope)、[微软 RichEdit 样式](https://learn.microsoft.com/en-us/windows/win32/controls/em-geteditstyle)。

## 验证边界

- Windows TSF 回归覆盖两个禁用标记的恢复，五种密码/PIN 范围、读锁失败和跨输入框切换。
- 私有桌面创建系统 `Msftedit.dll` 的 `RICHEDIT50W`，实际执行 FreeType 按键回调、显示候选和上屏。为避免注册或启用已安装输入法，该测试显式启用 RichEdit 的原生 TSF 模式。
- WinOne 的 Win32++ 适配使用默认 RichEdit 模式，未显式设置 `SES_USECTF`。该模式经过系统输入法兼容层；上述可控文本存储覆盖了相关状态与锁契约，但不等于已在 WinOne 实例现场验收。

已确认的状态残留缺陷与用户报告吻合。仍需用更新后的测试包在 WinOne 序号说明框完成现场复核，不能把原生 RichEdit 测试表述为 WinOne 端到端通过。
