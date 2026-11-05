# spanseek 第三方组件声明

本文件由 `node scripts/build-notices.mjs` 从 `Cargo.lock`、前端 `package.json` + 已安装包清单、`third-party/` 随包资产生成；**不要手改**，改动依赖后重跑。
许可文本本身随各组件自带的 LICENSE/NOTICE 文件分发；本文件只做索引与计数，不替代许可原文。

生成时间：2026-11-05T13:36:22.491Z

## 随包第三方资产

| 组件 | 版本 | 许可 |
| --- | --- | --- |
| Inter | 4.x (third-party/inter) | OFL-1.1（third-party/inter/LICENSE.txt） |
| tempfile (vendored crate) | 3.27.0 | MIT OR Apache-2.0（third-party/tempfile/LICENSE-*） |
| calamine (vendored crate, locally patched) | 0.37.0 | MIT（third-party/calamine/LICENSE-MIT.md + PATCHES.md） |
| mobi (vendored crate, locally patched) | 0.8.0 | MIT（third-party/mobi/LICENSE + NOTICE.md） |
| huffcdic (adapted from kindling-mobi) | 0.45.6 | MIT（third-party/mobi/LICENSE-kindling） |
| docx-preview | 0.4.1 | Apache-2.0（third-party/docx-preview/LICENSE） |
| JSZip | 3.10.2 | MIT (dual-licensed; MIT chosen)（third-party/jszip/LICENSE.markdown） |
| KaTeX fonts | katex-fonts (third-party/rich-format) | OFL-1.1（third-party/rich-format/katex-fonts/font-notices.json） |
| PDFium (pdfium-binaries chromium/7881) | 7881 | MIT（.local-archive/dependencies/pdfium-7881/LICENSE） |
| Tesseract OCR runtime + chi_sim+eng | 5.4.0.20240606 | Apache-2.0; bundled native components listed in COMPONENTS.md（third-party/tesseract/LICENSE + COMPONENTS.md + runtime-manifest.json） |

## Rust 依赖

共 661 个（来自 `cargo metadata --locked`，不含本仓库自身 crate）。

- MIT OR Apache-2.0: 306
- MIT: 142
- Apache-2.0 OR MIT: 59
- Unicode-3.0: 24
- MIT/Apache-2.0: 23
- Zlib OR Apache-2.0 OR MIT: 17
- Unlicense OR MIT: 11
- Apache-2.0: 9
- BSD-3-Clause: 7
- CC0-1.0: 6
- MPL-2.0: 5
- ISC: 5
- Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT: 5
- MIT OR Apache-2.0 OR Zlib: 4
- Zlib: 3
- Apache-2.0/MIT: 3
- Apache-2.0 OR ISC OR MIT: 3
- MIT OR Zlib OR Apache-2.0: 2
- BSD-3-Clause OR Apache-2.0: 2
- BSD-3-Clause OR MIT OR Apache-2.0: 2
- MIT OR Apache-2.0 OR LGPL-2.1-or-later: 2
- Unlicense/MIT: 2
- BSD-2-Clause OR Apache-2.0 OR MIT: 2
- 0BSD OR MIT OR Apache-2.0: 1
- ISC AND (Apache-2.0 OR ISC): 1
- ISC AND (Apache-2.0 OR ISC) AND Apache-2.0 AND MIT AND BSD-3-Clause AND (Apache-2.0 OR ISC OR MIT) AND (Apache-2.0 OR ISC OR MIT-0): 1
- BSD-3-Clause AND MIT: 1
- BSD-3-Clause/MIT: 1
- Apache-2.0 AND MIT: 1
- CC0-1.0 OR MIT-0 OR Apache-2.0: 1
- (Apache-2.0 OR MIT) AND BSD-3-Clause: 1
- MIT OR Apache-2.0 OR BSD-1-Clause: 1
- Apache-2.0 / MIT: 1
- MIT AND BSD-3-Clause: 1
- Apache-2.0 AND ISC: 1
- 0BSD OR CC0-1.0: 1
- Apache-2.0 OR BSL-1.0: 1
- Apache-2.0 WITH LLVM-exception: 1
- (MIT OR Apache-2.0) AND Unicode-3.0: 1
- CDLA-Permissive-2.0: 1

## 前端依赖

共 35 个（`apps/desktop` 与 `packages/contracts` 的直接依赖）。

- MIT: 28
- Apache-2.0: 3
- Apache-2.0 OR MIT: 2
- ISC: 2

## 富格式导出内置包（third-party/rich-format）

共 66 个；许可原文逐包保存在 `third-party/rich-format/<包>/LICENSE*`。

- see LICENSE: 41
- MIT: 19
- see LICENSE.txt: 4
- see LICENSE.md: 2

## 系统组件（不随安装包分发）

- Microsoft Edge WebView2 Runtime：由系统和安装器 bootstrap 提供，不随包复制。

## 未决

- 无。
