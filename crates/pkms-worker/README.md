# PKMS parser worker

单进程、单任务，stdin 是一个 EOF 结束的 JSON WorkerRequest（64 KiB），stdout 是一个 WorkerResponse（64 KiB）；日志仅错误码。应用签发并传递绝对输入文件、结果目录、DLL 路径，前端不能直接指定这些路径。进程无数据库连接，PDF 不执行 JavaScript/XFA；这是崩溃隔离，不是 OS 沙箱。

应用负责 120 秒超时、取消后 kill/wait、退出清理和租约续期。成功后调用 `read_validated_result` 校验 job/attempt/lease/source version、固定 `parsed.json` 文件名、SHA-256、文件类型和大小，再通过 core 验证并事务发布。失败和崩溃遗留 staging 不能发布；每个 attempt 使用全新专属目录，避免 `create_new` 覆盖。

硬限额：PDF 输入 256 MiB，MD/TXT 输入 32 MiB；PDF 10,000 页／200,000 Unicode 字符；结果 JSON 32 MiB；PDF 块 16 KiB。页内文字遵循 PDFium 字符顺序，不承诺复杂多栏语义重排。无文字页设置 `needs_ocr`（也可能是空白页）；不伪造 OCR。MD 保留原始 Markdown 文本行，并不将其伪装成完整富文本转换。无效 UTF-8、加密需要密码、损坏 PDF、缺少引擎、大小超限分别返回错误码。

PDF 几何保存未旋转 PDF 用户空间 `[left,bottom,right,top]`，CropBox 缺省时依 PDF 规范使用 MediaBox；页旋转单独保存。每个 Unicode code point 一条坐标，空白／退化框为 None，不声称精确定位。所有非私有真实 PDF 样本由测试生成。

## 固定依赖与许可证

- `pdfium-render = 0.9.4`，MIT OR Apache-2.0，关闭默认图像/线程特性，选择 `pdfium_7881` API。[源码/API](https://docs.rs/pdfium-render/0.9.4/pdfium_render/)、[仓库](https://github.com/ajrcarey/pdfium-render)。
- Windows x64 PDFium `151.0.7881.0`：[固定 release](https://github.com/bblanchon/pdfium-binaries/releases/tag/chromium%2F7881)。本次下载真实核查 VERSION 和 args.gn：V8=false、XFA=false、standalone=true、x64/windows。上游构建来自 bblanchon，不把社区预编译称为 Google 官方发行包。
- 下载：`pwsh -File scripts/fetch-pdfium.ps1`；默认只写被忽略的本机目录，不改 PATH/系统设置。包 SHA256 `73cc0de638ac2095e7445bf56a38200a5b7c7ca0e9f4ba144598f2457377ac08`；DLL SHA256 `79d4676b656cfb1abcea88f9ade3b4b0826c5200382db5f4ec72a636c598c118`。哈希来自本次下载锁定，不声称为上游签名认证。worker 启动前也核对 DLL 哈希，不搜索工作目录或系统 PATH。
- PDFium 自身为 BSD 三条款，具体文本位于包 `licenses/pdfium.txt`；包构建脚本 MIT 位于 `LICENSE`；其余 14 份 third-party 许可在 `licenses/`，含 ICU、FreeType、libjpeg、libpng 等。分发必须带整个 LICENSE + licenses 目录，不能只带 DLL 或只宣称全部 MIT。Google [PDFium 原始许可](https://pdfium.googlesource.com/pdfium/+/refs/heads/main/LICENSE)。
- 当前只验证 Windows x64。其他平台不能复用这个 DLL/hash，需要独立固定资源和验证。正式 Tauri sidecar/DLL/许可打包及干净机安装由桌面发布流程验收。

测试：`cargo test -p pkms-worker --locked`；原生 PDF 矩阵需 `PKMS_TEST_PDFIUM` 指向上述 DLL，然后 `cargo test -p pkms-worker --locked -- --ignored`。测试进程串行执行原生解析，不依赖 thread_safe 互斥。
