# 输入法技术文档

这些文档来自保留的 WowType 基线，用于理解已有引擎和平台接口。原文中的 Linux/macOS 安装流程、功能状态和测试结论不代表 FreeType 的 Windows 版本已实现或验证。

- [C ABI](ABI.md)：平台外壳调用引擎的接口。
- [移植边界](PORTING.md)：输入会话与平台集成的职责。
- [引擎说明](ENGINE.md)：拼音、候选词和排序实现。
- [数据来源](DATA.md)：词库、模型和数据格式。
- [语音说明](VOICE.md)：上游语音实现及限制。
- [已知限制](LIMITATIONS.md)：上游记录的能力边界。
- [使用说明](USAGE.md) 和 [配置说明](CONFIG.md)：功能对照，不是 Windows 操作指南。
- [上游安装说明](INSTALL.md)：保留构建信息，不在本轮执行安装。

Windows 版的结构、按键行为、水杉格式兼容、构建与验证范围见 [Windows 版说明](WINDOWS.md)；固定测试包约定见 [项目说明](../README.md)。

