# 来源阅读界面

`SourcesPage` 与 `SourceReader` 仅通过生成的桌面命令访问知识库。原件、解析和定位的权威数据均由 Rust 核心维护；这里不解析文件后写库。

- `SourcesPage`：资料列表、搜索与统一筛选、独立批量选择、导入弹窗和导入记录。主行打开原文，勾选与操作菜单独立；资料列表通过 `sources_browse` 从同一快照读取准确总数与编号页，直接跳页；返回/刷新恢复筛选、页码、跨页选择和可见条目锚点，末页缩短时按真实总数收缩。“全选本页”只增减本页，不覆盖其他页选择。高级筛选收进统一弹层并即时生效；窄窗口收起重复分类栏，仍能从筛选弹层选择分类。标签、集合候选继续使用既有服务端游标分页。导入期间的横幅除“正在导入资料…”外还显示本次批量的 `已完成 M / N 份`，收起弹窗后仍能看到进度。
- `SourceLibraryRow`：直接展示 `SourceSummary` 的原件名/网页地址、正文摘要和必要状态，不逐项请求正文。同名条目以格式、来源和短内容标识区分；阅读、菜单与勾选的可访问名称也包含这些信息。
- `useSourceImports`：文件批量选择后逐项受理；遇到未确认结果停止后续提交，保留原 command ID。网页受理使用同一任务队列。按真实 job ID 跟踪本次批次，超过最近 20 个任务时继续核对，额外读取并发最多 6。受理、解析成功、解析失败及等待确认分别显示，不从任务队列消失推断成功。
- `SourceReader`：TXT / Markdown / 网页使用固定解析版本的正文块，PDF 使用本地 PDF.js。版本、批注独立分页；正文回跳保留 source version、parse ID 与 block offset。`initialTarget` 接收核心已解析的来源目标。侧栏元信息的原件大小按 `source-size.ts` 的量级选单位（`512 B` / `1.6 KB` / `8.0 MB` / `1.5 GB`），不再把所有原件都写成 KB 计数。
- `SourceNoteImport`：MD/TXT 导入后可显式创建可编辑笔记；读取固定版本的不可变 UTF-8 原件（最多 1 MiB，核验 SHA-256），在 Web Worker 中复用 Markdown 转换器，不能使用已扁平化的检索正文。不支持的 Markdown 语法保留原文并提示。核心在同一事务创建富文本修订、原件版本引用、投影及回执；沿用 `CreateActions` 保留待确认请求和已创建未打开结果，编辑不写回原文件。
- `onAddToNote` 传递 1–100 条完整 `Annotation[]`；引用插入由应用层处理，不能仅保存 quote 文本。多选按当前列表顺序追加到同一笔记，并逐项保留原文、批注、固定来源版本与页码。`hideSidebar` 在双面板中让批注/信息替换当前阅读内容，避免第三面板。
- 摘录加入笔记复用共享 `DraftController`，一次 `update` + `flush` 保存整批。schema 2 批次意图先落本机 IndexedDB；旧 schema 1 单条意图保持可读。重开任一成员可继续原批次；回执未知沿用同一命令，部分摘录被移除时保留待办并要求打开笔记核对，不能静默重插。
- `active` 指示真正前台面板。`ReadingQueue` 拒绝旧 owner / 旧版本的晚到位置，失败保留最后位置。应用在切换、关闭、退出及一致性备份前必须 `await flushSourceReadings(identity)`，失败时保留当前界面。
- `SourceActions` 按知识库和来源共享，先以严格 IndexedDB 事务保存完整请求，再发送；未知结果重放同一 ID/版本。删除本机请求时在同一事务内核对 command ID，不能删除后来的请求。

## 资料入口与交互

`SourceImportDialog` 用独立的本地文件/网页页签与可选目标集合。取消文件选择不创建笔记或提交导入；任务提交期间收起弹窗会继续导入。原生 `<dialog>` 保留 Esc、回焦点和模态边界，集合使用全应用 `Combobox`，普通筛选使用 `Select`，标签浮层沿用统一的视区定位。

`SourceImportActivity` 显示本次及最近逐项结果，提供原请求核对、剩余文件继续、按固定版本打开原文与任务中心入口。本次导入先给总体进度（`已完成 M / N 份` + 各状态计数 + `role="progressbar"`），失败与取消按 `source-import-failure.ts` 的同一张码表分组汇总（`2 项未完成：处理超时 ×1 · 已取消 ×1`），最近导入把失败记录按同一组归到各自小标题下，成功的记录保持原顺序排在后面。解析失败、已在回收站的重复资料会提醒处理；“已查看结果”只收起已读提醒，不删除任务。密码 PDF、损坏/不可读文件、编码和大小限制有可理解的原因，原错误码仍可追溯。

应用层可传入 `importRequest: { id, grants }`，并通过 `onImportRequestHandled(id)` 清除请求入口。单文件/网页成功后用 `onOpenImportedSource(sourceId, versionId, inputToken)` 打开；`inputToken` 关联临时 PDF 的转存来源，供 App 保留原阅读位置。普通列表打开仍使用 `onOpenSource`。欢迎页的 `onOpenPdf` 是独立入口；临时阅读、拖入、快捷键和原生授权由 App/Tauri 接线，不能用浏览器受控回调代替原生验收。

## PDF

PDF.js 固定 `6.3.289`（Apache-2.0）。worker、CMaps、标准字体、WASM 和许可证随 Vite 构建本地输出，运行不依赖 CDN。原件票据只授权固定版本；custom range transport 验证 206、Content-Range 与长度，每次原生读取不超过 4 MiB。临时 PDF 与已入库来源共用 `PdfReader`；临时原生 URL 先 HEAD 获取长度，再走相同 Range transport，避免大于 16 MiB 时无界 GET 被端点拒绝。

原件票据/范围读取失败按已知安全错误码区分读取失败、原件或授权不可用、完整性校验失败等；不显示异常中的文件路径/票据 URL，也不把所有读取错误等同于原件丢失。错误区保留与当前固定版本匹配的摘录 quote 和页码，可重试读取同一版本。未知错误显示读取失败；后续销毁/取消错误不能覆盖首次读取失败的原因。

PDF 默认连续竖向阅读。所有页保留几何占位，可见窗口最多 12 页带 canvas/文字层，单 canvas backing pixels 约 800 万；移出窗口释放绘制资源。普通文档（最多 256 页）先并发读取轻量页几何，避免混合纸张、CropBox 或 UserUnit 让初次续读位置被临时滚动边界截断；更长文档按需补页几何。页码、目录、查找可进入尚未绘制的页面。查找逐页定位下一个匹配页，未提供全文命中数或全部匹配装饰。

文字层显式应用旋转、CropBox、UserUnit 与 PDF.js 缩放变量。选区通过 inverse viewport 转为 `pdf_user_space`；PDF 跨行/相邻已加载页按页保存 locator，跨未加载或无文字页会提示分段。原文高亮、页外旁注按钮和侧栏共用既有 annotation；相邻按钮聚合成计数菜单，不覆盖 PDF 正文。框选区域明确标记为无文字引文；`needs_ocr` 只表示文档中有页面无可提取文字，不把整份 PDF 描述为无文本层，也不宣称已有 OCR 能力。非 PDF 正文仍按单段摘录。

PDF 阅读区使用 `data-local-zoom="pdf"`，局部 Ctrl+滚轮/键盘缩放与工具栏共享 25%–500% 倍率和旋转状态；全局 WebView 缩放跳过此区域。`PdfReadingView` 保存页内归一化锚点、倍率和旋转。`positionReader` 在关闭屏障同步捕获最后视口，避免漏掉尚未派发的 scroll event；来源沿既有持久意图保存，解析未完成时仅提交 view、不伪造精确 locator。显式临时阅读位置优先于去重来源的旧读位。原件不被批注或缩放写回。

## 检查与参考

- `node --experimental-strip-types --test apps/desktop/tests/source-*.test.mjs`：Unicode 偏移、旋转坐标、未知结果重放、磁盘失败、读位关闭屏障和多面板 owner。
- `node --experimental-strip-types --test apps/desktop/tests/pdf-reader-errors.test.mjs`：原生错误分类、范围/解析错误与固定版本摘录快照匹配。
- `node --experimental-strip-types --test apps/desktop/tests/source-reading.test.mjs apps/desktop/tests/pdf-reading-geometry.test.mjs`：视口几何、旋转锚点、关闭屏障、倍率、失败重试和面板隔离。
- [PDF.js Range API](https://mozilla.github.io/pdf.js/api/draft/module-pdfjsLib-PDFDataRangeTransport.html)
- [Reader 高亮、批注与键盘行为](https://docs.readwise.io/reader/docs/faqs/highlights-tags-notes)

浏览器组件测试不能代替真实 Tauri 的原件缺失/恢复与保存屏障检查。
