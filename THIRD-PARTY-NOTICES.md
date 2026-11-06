# spanseek 第三方组件声明

由 `scripts/build-notices.mjs` 根据完整许可材料生成；修改依赖后先运行 `python scripts/build-license-bundle.py`。

安装目录 `licenses/dependencies/` 和 Release 附件均提供：

- `THIRD-PARTY-LICENSES.txt`：上游许可、版权与 NOTICE 全文。
- `THIRD-PARTY-COMPONENTS.json`：版本、许可文件 SHA-256、补充材料的来源。
- `THIRD-PARTY-SOURCES.zip`：MPL Rust 组件、Eigen、ELK/elkjs 的源码与构建说明。
- `THIRD-PARTY-SOURCE-INFO.txt`：适用许可与源码说明。

本索引覆盖已解析 Rust 依赖和已安装前端依赖，包括开发/构建工具；不表示每个包都进入安装包。第三方组件继续适用各自许可，不受 spanseek 专有许可限制。

## 原生组件与资产

- Tesseract 5.5.4：Apache-2.0；固定 vcpkg 源码构建，静态 MSVC CRT/库。完整依赖版权、源码哈希与端口补丁位于 `tesseract/licenses/`，实际文件哈希见 `tesseract/build-receipt.json`。
- chi_sim / eng：tessdata_fast 固定提交，Apache-2.0。
- PDFium chromium/7881：BSD-3-Clause；预编译包装项目为 MIT。本体及依赖许可证全文位于 `pdfium/licenses/`，包装许可为 `pdfium/LICENSE`。
- ONNX Runtime 1.28.0：MIT；完整第三方声明纳入许可文本，Eigen 对应源码附在源码包中。
- Inter / KaTeX 字体：OFL-1.1；随包保留原始字体许可。
- 本仓库修改过的 tempfile、calamine、mobi 保留原许可与补丁说明，源码随仓库公开。
- DOMPurify 按 Apache-2.0 选项使用；JSZip 按 MIT 选项使用。
- WebView2 由系统或安装器 bootstrap 提供，不在本包复制其运行库。

## 依赖许可声明统计

Rust：735；npm：421。

- (Apache-2.0 OR MIT) AND BSD-3-Clause: 1
- (MIT AND Zlib): 1
- (MIT OR Apache-2.0) AND Apache-2.0: 1
- (MIT OR Apache-2.0) AND Unicode-3.0: 1
- (MIT OR GPL-3.0-or-later): 1
- (MPL-2.0 OR Apache-2.0): 1
- 0BSD OR CC0-1.0: 1
- 0BSD OR MIT OR Apache-2.0: 1
- Apache-2.0: 101
- Apache-2.0 / MIT: 1
- Apache-2.0 AND ISC: 1
- Apache-2.0 AND MIT: 2
- Apache-2.0 OR BSL-1.0: 1
- Apache-2.0 OR ISC OR MIT: 3
- Apache-2.0 OR MIT: 53
- Apache-2.0 OR MIT OR Zlib: 2
- Apache-2.0/MIT: 3
- BSD-2-Clause: 4
- BSD-2-Clause OR Apache-2.0 OR MIT: 2
- BSD-3-Clause: 18
- BSD-3-Clause AND MIT: 1
- BSD-3-Clause OR Apache-2.0: 2
- BSD-3-Clause/MIT: 1
- BSL-1.0: 1
- CC0-1.0: 7
- CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception: 1
- CC0-1.0 OR MIT-0 OR Apache-2.0: 2
- CDLA-Permissive-2.0: 1
- EPL-2.0: 1
- ISC: 50
- ISC AND (Apache-2.0 OR ISC): 1
- ISC AND (Apache-2.0 OR ISC) AND Apache-2.0 AND MIT AND BSD-3-Clause AND (Apache-2.0 OR ISC OR MIT) AND (Apache-2.0 OR ISC OR MIT-0): 1
- MIT: 495
- MIT AND BSD-3-Clause: 2
- MIT OR Apache-2.0: 295
- MIT OR Apache-2.0 OR Zlib: 4
- MIT OR Zlib OR Apache-2.0: 2
- MIT/Apache-2.0: 32
- MPL-2.0: 7
- Unicode-3.0: 26
- Unlicense: 1
- Unlicense OR MIT: 11
- Unlicense/MIT: 6
- Zlib: 4
- Zlib OR Apache-2.0 OR MIT: 3

## 材料完整性检查

缺失许可文本：0。构建检查校验补充文本哈希、原生来源、对应源码归档哈希和未审阅的新许可；本清单不是法律意见。
