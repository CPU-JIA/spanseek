#!/usr/bin/env node
/** Build the notice index from the complete, verified license-text bundle. */
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

const root = fileURLToPath(new URL('../', import.meta.url));
const inventory = JSON.parse(readFileSync(join(root, '.local-archive/license-bundle/THIRD-PARTY-COMPONENTS.json'), 'utf8'));
if (inventory.missing.length) throw new Error('Complete license evidence is missing');
const rust = inventory.packages.filter((entry) => entry.id.startsWith('cargo:'));
const npm = inventory.packages.filter((entry) => entry.id.startsWith('npm:'));
const counts = new Map();
for (const entry of [...rust, ...npm]) counts.set(entry.license, (counts.get(entry.license) ?? 0) + 1);
const lines = [
  '# spanseek 第三方组件声明',
  '',
  '由 `scripts/build-notices.mjs` 根据完整许可材料生成；修改依赖后先运行 `python scripts/build-license-bundle.py`。',
  '',
  '安装目录 `licenses/dependencies/` 和 Release 附件均提供：',
  '',
  '- `THIRD-PARTY-LICENSES.txt`：上游许可、版权与 NOTICE 全文。',
  '- `THIRD-PARTY-COMPONENTS.json`：版本、许可文件 SHA-256、补充材料的来源。',
  '- `THIRD-PARTY-SOURCES.zip`：MPL Rust 组件、Eigen、ELK/elkjs 的源码与构建说明。',
  '- `THIRD-PARTY-SOURCE-INFO.txt`：适用许可与源码说明。',
  '',
  '本索引覆盖已解析 Rust 依赖和已安装前端依赖，包括开发/构建工具；不表示每个包都进入安装包。第三方组件继续适用各自许可，不受 spanseek 专有许可限制。',
  '',
  '## 原生组件与资产',
  '',
  '- Tesseract 5.5.4：Apache-2.0；固定 vcpkg 源码构建，静态 MSVC CRT/库。完整依赖版权、源码哈希与端口补丁位于 `tesseract/licenses/`，实际文件哈希见 `tesseract/build-receipt.json`。',
  '- chi_sim / eng：tessdata_fast 固定提交，Apache-2.0。',
  '- PDFium chromium/7881：BSD-3-Clause；预编译包装项目为 MIT。本体及依赖许可证全文位于 `pdfium/licenses/`，包装许可为 `pdfium/LICENSE`。',
  '- ONNX Runtime 1.28.0：MIT；完整第三方声明纳入许可文本，Eigen 对应源码附在源码包中。',
  '- Inter / KaTeX 字体：OFL-1.1；随包保留原始字体许可。',
  '- 本仓库修改过的 tempfile、calamine、mobi 保留原许可与补丁说明，源码随仓库公开。',
  '- DOMPurify 按 Apache-2.0 选项使用；JSZip 按 MIT 选项使用。',
  '- WebView2 由系统或安装器 bootstrap 提供，不在本包复制其运行库。',
  '',
  '## 依赖许可声明统计',
  '',
  `Rust：${rust.length}；npm：${npm.length}。`,
  '',
  ...[...counts.entries()].sort(([a], [b]) => a.localeCompare(b)).map(([license, count]) => `- ${license}: ${count}`),
  '',
  '## 材料完整性检查',
  '',
  '缺失许可文本：0。构建检查校验补充文本哈希、原生来源、对应源码归档哈希和未审阅的新许可；本清单不是法律意见。',
  '',
];
writeFileSync(join(root, 'THIRD-PARTY-NOTICES.md'), lines.join('\n'));
console.log(JSON.stringify({ rust: rust.length, npm: npm.length, missing: inventory.missing.length }));
