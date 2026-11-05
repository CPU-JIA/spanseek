#!/usr/bin/env node
/**
 * 生成 THIRD-PARTY-NOTICES.md：Rust 依赖 + 前端依赖 + 随包第三方资产。
 *
 * 规则：
 * - 只从锁文件与已安装包的清单读事实，不猜许可；
 * - 任何一项读不到许可就列进“未决”并以非零退出（发布前必须清零）；
 * - 许可文本本身由各包自带的 LICENSE 文件随包分发，本文件只做索引。
 */
import { execFileSync } from 'node:child_process';
import { readFileSync, readdirSync, existsSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const read = (path) => readFileSync(path, 'utf8');
const readJson = (path) => JSON.parse(read(path));
const exists = (path) => existsSync(path);

const unknown = [];
const rust = new Map();
const metadata = JSON.parse(
  execFileSync('cargo', ['metadata', '--format-version', '1', '--locked'], {
    cwd: root,
    encoding: 'utf8',
    maxBuffer: 256 * 1024 * 1024,
  }),
);
const workspace = new Set(metadata.workspace_members);
for (const pkg of metadata.packages) {
  if (workspace.has(pkg.id) || !pkg.source) continue;
  const key = `${pkg.name}@${pkg.version}`;
  const license = pkg.license || (pkg.license_file ? 'see LICENSE file' : null);
  if (!license) unknown.push(`rust ${key}`);
  rust.set(key, license ?? 'UNKNOWN');
}

const npm = new Map();
const projects = ['apps/desktop/package.json', 'packages/contracts/package.json'];
for (const project of projects) {
  if (!exists(join(root, project))) continue;
  const manifest = readJson(join(root, project));
  for (const name of Object.keys({ ...manifest.dependencies, ...manifest.devDependencies })) {
    if (name.startsWith('@pkms/')) continue;
    const candidates = [
      join(root, 'node_modules', name, 'package.json'),
      join(root, dirname(project), 'node_modules', name, 'package.json'),
    ];
    const path = candidates.find(exists);
    if (!path) {
      unknown.push(`npm ${name} (not installed)`);
      continue;
    }
    const pkg = readJson(path);
    const license = pkg.license || pkg.licenses?.map?.((entry) => entry.type).join(' OR ') || null;
    if (!license) unknown.push(`npm ${name}@${pkg.version}`);
    npm.set(`${name}@${pkg.version}`, license ?? 'UNKNOWN');
  }
}

const assets = [];
assets.push(['Inter', '4.x (third-party/inter)', 'OFL-1.1', 'third-party/inter/LICENSE.txt']);
if (exists(join(root, 'third-party/tempfile/Cargo.toml'))) {
  const tempfileToml = read(join(root, 'third-party/tempfile/Cargo.toml'));
  const tempfileVersion = tempfileToml.match(/^version\s*=\s*"([^"]+)"/m)?.[1] ?? '?';
  assets.push([
    'tempfile (vendored crate)',
    tempfileVersion,
    'MIT OR Apache-2.0',
    'third-party/tempfile/LICENSE-*',
  ]);
}
if (exists(join(root, 'third-party/calamine/Cargo.toml'))) {
  const calamineToml = read(join(root, 'third-party/calamine/Cargo.toml'));
  const calamineVersion = calamineToml.match(/^version\s*=\s*"([^"]+)"/m)?.[1] ?? '?';
  assets.push([
    'calamine (vendored crate, locally patched)',
    calamineVersion,
    'MIT',
    'third-party/calamine/LICENSE-MIT.md + PATCHES.md',
  ]);
}
if (exists(join(root, 'third-party/mobi/Cargo.toml'))) {
  const mobiToml = read(join(root, 'third-party/mobi/Cargo.toml'));
  const mobiVersion = mobiToml.match(/^version\s*=\s*"([^"]+)"/m)?.[1] ?? '?';
  assets.push([
    'mobi (vendored crate, locally patched)',
    mobiVersion,
    'MIT',
    'third-party/mobi/LICENSE + NOTICE.md',
  ]);
  assets.push([
    'huffcdic (adapted from kindling-mobi)',
    '0.45.6',
    'MIT',
    'third-party/mobi/LICENSE-kindling',
  ]);
}
const bundledFrontend = [
  ['docx-preview', 'docx-preview', 'third-party/docx-preview/LICENSE', null],
  ['JSZip', 'jszip', 'third-party/jszip/LICENSE.markdown', 'MIT (dual-licensed; MIT chosen)'],
];
for (const [label, packageName, licensePath, licenseOverride] of bundledFrontend) {
  if (!exists(join(root, licensePath))) continue;
  const direct = [
    join(root, 'apps/desktop/node_modules', packageName, 'package.json'),
    join(root, 'node_modules', packageName, 'package.json'),
  ].find(exists);
  const store = join(root, 'node_modules/.pnpm');
  const stored = exists(store)
    ? readdirSync(store)
        .filter((entry) => entry.startsWith(`${packageName}@`))
        .sort()
        .at(-1)
    : undefined;
  const manifest =
    direct ?? (stored ? join(store, stored, 'node_modules', packageName, 'package.json') : null);
  if (!manifest || !exists(manifest)) {
    unknown.push(`asset ${label} (not installed)`);
    continue;
  }
  const pkg = readJson(manifest);
  assets.push([label, pkg.version, licenseOverride ?? pkg.license ?? 'see LICENSE', licensePath]);
}
const richDir = join(root, 'third-party/rich-format');
const rich = new Map();
if (exists(richDir)) {
  for (const entry of readdirSync(richDir, { withFileTypes: true })) {
    if (!entry.isDirectory() || !entry.name.includes('@')) continue;
    const pkgPath = join(richDir, entry.name, 'package.json');
    const pkg = exists(pkgPath) ? readJson(pkgPath) : null;
    const name = entry.name.replace(/@(\d|\.)+$/, '');
    const version = entry.name.match(/@(\d[\w.-]*)$/)?.[1] ?? pkg?.version ?? '?';
    let license = pkg?.license ?? null;
    if (!license) {
      const files = readdirSync(join(richDir, entry.name));
      const licenseFile = files.find((name) => /^licen[cs]e/i.test(name));
      if (licenseFile) {
        const head = read(join(richDir, entry.name, licenseFile));
        license = /MIT License/i.test(head) ? 'MIT' : `see ${licenseFile}`;
      }
    }
    if (!license) unknown.push(`asset ${entry.name}`);
    rich.set(`${name}@${version}`, license ?? 'UNKNOWN');
  }
}
if (exists(join(richDir, 'katex-fonts'))) {
  assets.push([
    'KaTeX fonts',
    'katex-fonts (third-party/rich-format)',
    'OFL-1.1',
    'third-party/rich-format/katex-fonts/font-notices.json',
  ]);
}
const pdfium = join(root, '.local-archive/dependencies/pdfium-7881/LICENSE');
if (exists(pdfium)) {
  assets.push([
    'PDFium (pdfium-binaries chromium/7881)',
    '7881',
    'MIT',
    '.local-archive/dependencies/pdfium-7881/LICENSE',
  ]);
}
const tesseractManifestPath = join(root, 'third-party/tesseract/runtime-manifest.json');
if (exists(tesseractManifestPath)) {
  const tesseract = readJson(tesseractManifestPath);
  assets.push([
    `Tesseract OCR runtime + ${tesseract.tessdata.files.map((file) => file.path.replace('.traineddata', '')).join('+')}`,
    tesseract.version,
    'Apache-2.0; bundled native components listed in COMPONENTS.md',
    'third-party/tesseract/LICENSE + COMPONENTS.md + runtime-manifest.json',
  ]);
}

const table = (rows) => ['| 组件 | 版本 | 许可 |', '| --- | --- | --- |', ...rows].join('\n');
const sorted = (map) => [...map.entries()].sort((a, b) => a[0].localeCompare(b[0]));
const count = (map) => {
  const out = new Map();
  for (const license of map.values()) out.set(license, (out.get(license) ?? 0) + 1);
  return [...out.entries()].sort((a, b) => b[1] - a[1]);
};

const lines = [
  '# spanseek 第三方组件声明',
  '',
  '本文件由 `node scripts/build-notices.mjs` 从 `Cargo.lock`、前端 `package.json` + 已安装包清单、`third-party/` 随包资产生成；**不要手改**，改动依赖后重跑。',
  '许可文本本身随各组件自带的 LICENSE/NOTICE 文件分发；本文件只做索引与计数，不替代许可原文。',
  '',
  `生成时间：${new Date().toISOString()}`,
  '',
  '## 随包第三方资产',
  '',
  table(
    assets.map(
      ([name, version, license, path]) => `| ${name} | ${version} | ${license}（${path}） |`,
    ),
  ),
  '',
  '## Rust 依赖',
  '',
  `共 ${rust.size} 个（来自 \`cargo metadata --locked\`，不含本仓库自身 crate）。`,
  '',
  count(rust)
    .map(([license, value]) => `- ${license}: ${value}`)
    .join('\n'),
  '',
  '## 前端依赖',
  '',
  `共 ${npm.size} 个（\`apps/desktop\` 与 \`packages/contracts\` 的直接依赖）。`,
  '',
  count(npm)
    .map(([license, value]) => `- ${license}: ${value}`)
    .join('\n'),
  '',
  '## 富格式导出内置包（third-party/rich-format）',
  '',
  `共 ${rich.size} 个；许可原文逐包保存在 \`third-party/rich-format/<包>/LICENSE*\`。`,
  '',
  count(rich)
    .map(([license, value]) => `- ${license}: ${value}`)
    .join('\n'),
  '',
  '## 系统组件（不随安装包分发）',
  '',
  '- Microsoft Edge WebView2 Runtime：由系统和安装器 bootstrap 提供，不随包复制。',
  '',
  '## 未决',
  '',
  unknown.length === 0 ? '- 无。' : unknown.map((entry) => `- ${entry}`).join('\n'),
  '',
];
writeFileSync(join(root, 'THIRD-PARTY-NOTICES.md'), lines.join('\n'));
console.log(
  JSON.stringify(
    {
      rust: rust.size,
      npm: npm.size,
      rich: rich.size,
      assets: assets.length,
      unknown: unknown.length,
      unknownList: unknown,
    },
    null,
    2,
  ),
);
if (unknown.length) process.exitCode = 1;
