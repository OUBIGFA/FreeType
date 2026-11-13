# 输入法技术文档

[English](README.en.md) | 简体中文

这些文档说明 FreeType 使用的 WowType 核心、C ABI 和 Windows 外壳。Linux/macOS 的原始安装流程和功能状态只用于技术对照，不代表 FreeType 的平台支持承诺。

- [Windows 版说明](WINDOWS.md)：Windows 结构、输入行为、词库导入、数据目录、构建和验证范围。
- [C ABI](ABI.md)：平台外壳调用 Rust 核心的接口。
- [移植边界](PORTING.md)：输入会话和平台集成的职责。
- [引擎说明](ENGINE.md)：拼音、候选词和排序实现。
- [数据来源](DATA.md)：词库、模型和数据格式。
- [语音说明](VOICE.md)：上游语音实现及其限制。
- [已知限制](LIMITATIONS.md)：上游实现记录的能力边界。
- [使用说明](USAGE.md) 和 [配置说明](CONFIG.md)：核心功能和配置对照。
- [上游安装说明](INSTALL.md)：保留的 Linux 构建信息，不是 Windows 安装指南。

固定测试包约定见 [项目说明](../README.md)。第三方来源和许可状态见 [第三方来源与许可](../third_party/README.md)。
