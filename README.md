<p align="center">
  <img src="docs/download/assets/icon.svg" width="88" height="88" alt="跨索 spanseek" />
</p>

# 跨索 spanseek

**跨索（spanseek）是一个把原件、摘录、笔记和 AI 结论都可追溯地放在一起的本地知识库。AI 只能提案，不能改你的正文。**

> 让每一次求索，都有迹可循。

![跨索 spanseek 编辑器](docs/download/assets/editor.png)

跨索 spanseek 面向个人学习、研究与写作。它不要求你把资料上传到云端，也不把“收集”当成终点：每一条判断都应该能回到它的出处、上下文与版本。

## 为什么是跨索

- **原件不可变**：导入的 PDF、Word（DOCX/DOC）、Excel（XLSX/XLS）、PowerPoint（PPTX）、EPUB/MOBI、CSV、Markdown 与 TXT 按内容寻址保存，编辑只发生在笔记与修订中。
- **结论可追溯**：双链、反链、摘录与引用把笔记和原始材料连在一起。
- **AI 只提案**：生成内容先成为提案，只有你核对并明确应用后才会写入。
- **默认留在本机**：不配置外部服务时，记录、阅读、检索与完整备份都可离线完成。
- **数据带得走**：完整备份可恢复到新目录，也可以导出 Markdown / Obsidian 兼容目录。

## 能做什么

- **读**：PDF 目录、页码、缩放、适宽、摘录与阅读位置；Word、Excel、PowerPoint 的版式/内容阅读；EPUB/MOBI 封面、目录与正文图片；CSV、Markdown 与 TXT 阅读及可摘录原文。
- **写**：富格式笔记、标题折叠、任务列表、表格、图片、公式与 Mermaid 图表。
- **连**：双向链接、反链上下文、对象关系、集合、标签、属性、表格、看板、日历、时间线与主题画布。
- **找**：中文全文检索、按对象类型筛选、关键词高亮、最近搜索与可选语义检索；未配置语义服务时明确回退到关键词。
- **问**：自由对话、严格依证问答和知识任务；从材料发起、自定义任务或可选模板起步，核对来源后审阅保存，保留原始结果与历史。
- **记**：语音速记、原音频回放、转写与手工修订、字幕导出；手机提供配套录音与捕获。
- **带走**：完整备份、校验、恢复到新目录，以及 Markdown / Obsidian 兼容导出。

## 下载

当前版本：**1.0.2**（Windows 10 / 11 x64），已公开发布。安装包由 GitHub Actions 构建，并完成隔离安装、关键功能与卸载验证。

- [下载 1.0.2 安装包](https://github.com/CPU-JIA/spanseek/releases/download/v1.0.2/spanseek_1.0.2_x64-setup.exe) · [发行说明与全部校验值](https://github.com/CPU-JIA/spanseek/releases/tag/v1.0.2)
- [下载页](docs/download/index.html)：安装包、SHA-256、系统要求与校验方法。
- [快速上手](docs/quick-start.md)：十分钟完成安装、建库、第一篇笔记、导入与备份。
- [已知问题](docs/known-issues.md)：未签名、WebView2、DOCX/PDF、AI 与未覆盖环境。

安装包未签名；Windows 可能显示“未知发布者”或 SmartScreen 提示。请先核对下载页上的 SHA-256。

## 系统要求

- Windows 10 1809 或更高版本，或 Windows 11，64 位
- Microsoft Edge WebView2 Runtime（Windows 11 通常已内置）
- 4 GB 内存；建议预留 200 MB 磁盘空间用于程序，知识库另计
- 核心功能无需联网；AI 与同步只有在你显式配置后才会连接外部服务

## 文档

- [产品演示](docs/download/index.html#motion)：50 秒有声演示，看懂原件、摘录、笔记、检索、AI 提案与可回查答案。
- [品牌故事](docs/download/brand.html)
- [用户指南](docs/user-guide.md)
- [隐私说明](docs/privacy.md)
- [版本记录](CHANGELOG.md)
- [第三方组件声明](THIRD-PARTY-NOTICES.md)
- [使用许可](LICENSE)

## 开发

开发环境：Windows x64、PowerShell 7、Rust/Cargo 1.98.0、Node 24.19.0、pnpm 10.25.0、Visual Studio C++ 构建工具和 WebView2；首次提取 OCR 运行库还需要 7-Zip。

```powershell
pnpm install --frozen-lockfile
pnpm desktop
```

资源准备脚本从固定来源获取 PDFium、protoc、OCR 运行库与语言包，并核对 SHA-256。外部资源和构建缓存不会提交到 Git。开发启动、完整检查、隔离测试与目录说明见 [开发指南](docs/development.md)；Actions、缓存和 tag 发布流程见 [构建与发布](docs/releasing.md)。

```powershell
pnpm desktop:build
```

桌面端采用 Tauri 2 + React/TypeScript，核心逻辑在独立 Rust 工作区。`pkms-core` 不依赖 Tauri 或 UI；前端不直接写数据库；共享 DTO 由 Rust 生成 TypeScript。

## 隐私与许可

- [隐私说明](docs/privacy.md)：默认数据流、外部服务、凭据与诊断。
- [使用许可](LICENSE)：免费使用，保留所有权利；第三方组件按其各自许可分发。

跨索 spanseek 目前不提供自动更新、代码签名或托管云服务。升级前请创建完整备份。

## 联系与支持

使用问题与反馈：[support@spanseek.com](mailto:support@spanseek.com)。
