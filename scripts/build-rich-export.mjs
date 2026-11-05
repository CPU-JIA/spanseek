import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync,
  realpathSync, renameSync, rmSync, statSync, writeFileSync,
} from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const desktop = path.join(root, 'apps/desktop');
const requireDesktop = createRequire(path.join(desktop, 'package.json'));
const { build } = await import(pathToFileURL(requireDesktop.resolve('vite')).href);
const requireVite = createRequire(requireDesktop.resolve('vite'));
const { parseSync } = await import(pathToFileURL(requireVite.resolve('rolldown/utils')).href);
const resources = path.join(root, 'crates/pkms-core/resources');
const installedLicenses = path.join(root, 'third-party/rich-format');
const archivePath = path.join(resources, 'rich-export.zip');
const sidecarPath = path.join(resources, 'rich-export.manifest.json');
const fontLicense = path.join(desktop, 'src/export-renderer/licenses/KaTeX-fonts-OFL.txt');
const sourceFiles = [
  'scripts/build-rich-export.mjs', 'package.json', 'pnpm-lock.yaml',
  'apps/desktop/package.json', 'apps/desktop/src/export-renderer/entry.ts',
  'apps/desktop/src/export-renderer/style.css',
  'apps/desktop/src/export-renderer/licenses/KaTeX-fonts-OFL.txt',
  'apps/desktop/src/rich/math-render.ts',
  'apps/desktop/src/rich/mermaid-render.ts',
  'apps/desktop/src/rich/render-policy.ts',
];
const prefix = 'assets/rich-export/';
const moduleIds = new Set();
const digest = (bytes) => createHash('sha256').update(bytes).digest('hex');
const posix = (value) => value.split(path.sep).join('/');
const sourceInputs = sourceFiles.map((name) => fileRecord(path.join(root, name), name));
const temporary = realpathSync(mkdtempSync(path.join(tmpdir(), 'pkms-rich-export-')));
const staged = path.join(temporary, 'bundle');
const pendingArchive = path.join(temporary, 'rich-export.zip');
const pendingSidecar = path.join(temporary, 'rich-export.manifest.json');

function fileRecord(filename, name) {
  const bytes = readFileSync(filename);
  return { path: name, bytes: bytes.length, sha256: digest(bytes) };
}

function inside(directory, filename) {
  const relative = path.relative(path.resolve(directory), path.resolve(filename));
  return relative !== '..' && !relative.startsWith('..' + path.sep) && !path.isAbsolute(relative);
}

function filesUnder(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((item) => {
    const filename = path.join(directory, item.name);
    if (item.isSymbolicLink()) throw new Error('Resource staging must not contain symlinks.');
    return item.isDirectory() ? filesUnder(filename) : item.isFile() ? [filename] : [];
  }).sort();
}

function writeStaged(relative, bytes) {
  const destination = path.resolve(staged, relative);
  if (!inside(staged, destination)) throw new Error('Invalid staged resource path.');
  mkdirSync(path.dirname(destination), { recursive: true });
  writeFileSync(destination, bytes);
}

function writeIfChanged(filename, bytes) {
  if (existsSync(filename) && readFileSync(filename).equals(bytes)) return false;
  mkdirSync(path.dirname(filename), { recursive: true });
  const pending = path.join(path.dirname(filename), '.' + path.basename(filename) + '.' + process.pid + '.tmp');
  try {
    writeFileSync(pending, bytes, { flag: 'wx' });
    renameSync(pending, filename);
  } finally {
    if (existsSync(pending)) rmSync(pending);
  }
  return true;
}

const packageCache = new Map();
function packageFor(filename) {
  let directory = path.dirname(filename);
  const visited = [];
  while (directory !== path.dirname(directory)) {
    if (packageCache.has(directory)) {
      const found = packageCache.get(directory);
      for (const item of visited) packageCache.set(item, found);
      return found;
    }
    visited.push(directory);
    const manifest = path.join(directory, 'package.json');
    if (existsSync(manifest)) {
      const metadata = JSON.parse(readFileSync(manifest, 'utf8'));
      if (metadata.name && metadata.version) {
        const found = { directory, metadata, manifest };
        for (const item of visited) packageCache.set(item, found);
        return found;
      }
    }
    directory = path.dirname(directory);
  }
  throw new Error('Cannot attribute bundled module: ' + path.basename(filename));
}

function fontNames(filename) {
  const data = readFileSync(filename);
  const count = data.readUInt16BE(4);
  let offset;
  for (let index = 0; index < count; index++) {
    const table = 12 + index * 16;
    if (data.toString('ascii', table, table + 4) === 'name') offset = data.readUInt32BE(table + 8);
  }
  if (offset === undefined) throw new Error('Font is missing its name table.');
  const records = data.readUInt16BE(offset + 2);
  const strings = data.readUInt16BE(offset + 4);
  const names = new Map();
  for (let index = 0; index < records; index++) {
    const record = offset + 6 + index * 12;
    const platform = data.readUInt16BE(record);
    const id = data.readUInt16BE(record + 6);
    if (![0, 13, 14].includes(id)) continue;
    const length = data.readUInt16BE(record + 8);
    const start = offset + strings + data.readUInt16BE(record + 10);
    const raw = Buffer.from(data.subarray(start, start + length));
    const value = platform === 0 || platform === 3 ? raw.swap16().toString('utf16le') : raw.toString('latin1');
    if (!names.has(id)) names.set(id, new Set());
    names.get(id).add(value);
  }
  return Object.fromEntries([...names].map(([id, values]) => [id, [...values].sort()]));
}

try {
  await build({
    configFile: false,
    root: desktop,
    publicDir: false,
    logLevel: 'warn',
    plugins: [{
      name: 'pkms-rich-export-inputs',
      generateBundle(_options, bundle) {
        for (const output of Object.values(bundle)) {
          if (output.type !== 'chunk') continue;
          // Rolldown can report an inlined self-edge in dynamicImports.
          if (output.imports.length || output.dynamicImports.some((name) => name !== output.fileName))
            throw new Error('Offline export must not contain external or dynamic JS imports: ' + JSON.stringify({ file: output.fileName, imports: output.imports, dynamicImports: output.dynamicImports }));
          for (const id of Object.keys(output.modules)) moduleIds.add(id);
        }
      },
    }],
    build: {
      outDir: staged,
      emptyOutDir: true,
      copyPublicDir: false,
      sourcemap: false,
      minify: 'oxc',
      cssCodeSplit: false,
      assetsInlineLimit: () => true,
      reportCompressedSize: false,
      target: 'es2022',
      lib: {
        entry: path.join(desktop, 'src/export-renderer/entry.ts'),
        name: 'PkmsRichExport',
        formats: ['iife'],
        fileName: () => 'renderer.js',
        cssFileName: 'style',
      },
      // Vite 8/Rolldown's equivalent of Rollup inlineDynamicImports: true.
      rolldownOptions: { output: { codeSplitting: false } },
    },
  });

  const built = filesUnder(staged).map((filename) => posix(path.relative(staged, filename)));
  if (built.length !== 2 || !built.includes('renderer.js') || !built.includes('style.css'))
    throw new Error('Expected exactly one classic JS bundle and one CSS bundle: ' + built.join(', '));
  const parsed = parseSync('renderer.js', readFileSync(path.join(staged, 'renderer.js'), 'utf8'), { sourceType: 'script' });
  if (parsed.errors.length) throw new Error('Export bundle is not a valid classic script.');
  const pendingNodes = [parsed.program];
  while (pendingNodes.length) {
    const node = pendingNodes.pop();
    if (node.type === 'ImportExpression' || node.type === 'ImportDeclaration' || node.type?.startsWith('Export'))
      throw new Error('Export bundle retained a module import or export.');
    for (const value of Object.values(node)) {
      if (Array.isArray(value)) pendingNodes.push(...value.filter((item) => item && typeof item === 'object'));
      else if (value && typeof value === 'object') pendingNodes.push(value);
    }
  }
  const css = readFileSync(path.join(staged, 'style.css'), 'utf8');
  for (const match of css.matchAll(/url\(\s*["']?([^)"']+)/gi)) {
    if (!match[1].startsWith('data:') && !match[1].startsWith('#'))
      throw new Error('CSS contains a non-embedded resource URL.');
  }

  const packages = new Map();
  const moduleInputs = [];
  for (const id of [...moduleIds].sort()) {
    if (id.startsWith('\0')) continue;
    const filename = id.split('?')[0];
    if (!path.isAbsolute(filename) || !existsSync(filename) || !statSync(filename).isFile()) continue;
    if (!posix(filename).includes('/node_modules/')) {
      if (!inside(root, filename)) throw new Error('Bundled source lies outside the workspace.');
      moduleInputs.push(fileRecord(filename, posix(path.relative(root, filename))));
      continue;
    }
    const owner = packageFor(filename);
    const key = owner.metadata.name + '@' + owner.metadata.version;
    const packageEntry = packages.get(key) ?? { ...owner, modules: 0 };
    packageEntry.modules++;
    packages.set(key, packageEntry);
    moduleInputs.push(fileRecord(filename, 'npm:' + key + '/' + posix(path.relative(owner.directory, filename))));
  }

  const dependencies = [];
  for (const [key, owner] of [...packages].sort(([a], [b]) => a.localeCompare(b))) {
    const licenseFiles = readdirSync(owner.directory, { withFileTypes: true })
      .filter((file) => file.isFile() && /^(licen[sc]e|copying|notice|copyright)(?:[._-].*)?$/i.test(file.name));
    if (licenseFiles.length === 0) throw new Error('Bundled package has no license text: ' + key);
    const licenses = licenseFiles.sort((a, b) => a.name.localeCompare(b.name)).map((file) => {
      const output = 'licenses/' + key.replaceAll('/', '__') + '/' + file.name;
      const bytes = readFileSync(path.join(owner.directory, file.name));
      writeStaged(output, bytes);
      return { ...fileRecord(path.join(owner.directory, file.name), prefix + output), source: file.name };
    });
    dependencies.push({
      name: owner.metadata.name, version: owner.metadata.version,
      license: owner.metadata.license ?? null, modules: owner.modules,
      packageJsonSha256: digest(readFileSync(owner.manifest)), licenseFiles: licenses,
    });
  }

  const katexRoot = packageFor(requireDesktop.resolve('katex')).directory;
  const katexCss = path.join(katexRoot, 'dist/katex.min.css');
  const fontPaths = [...new Set([...readFileSync(katexCss, 'utf8').matchAll(/url\(["']?([^)"']+)["']?\)/g)]
    .map((match) => path.resolve(path.dirname(katexCss), match[1])))].sort();
  const fonts = fontPaths.map((filename) => {
    if (!inside(path.join(katexRoot, 'dist/fonts'), filename)) throw new Error('Unexpected KaTeX CSS asset.');
    return fileRecord(filename, 'katex/dist/fonts/' + path.basename(filename));
  });
  const fontNotices = fontPaths.filter((filename) => filename.endsWith('.ttf')).map((filename) => {
    const names = fontNames(filename);
    if (!names[13]?.some((notice) => notice.includes('SIL Open Font License, Version 1.1')))
      throw new Error('Font license changed; review before redistributing: ' + path.basename(filename));
    return { font: path.basename(filename), copyright: names[0] ?? [], license: names[13], licenseUrl: names[14] ?? [] };
  });
  if (fontNotices.length !== 20) throw new Error('Unexpected KaTeX font set; review its licenses.');
  writeStaged('licenses/katex-fonts/OFL.txt', readFileSync(fontLicense));
  writeStaged('licenses/katex-fonts/font-notices.json', JSON.stringify(fontNotices, null, 2) + '\n');
  writeStaged('licenses/README.md', [
    '# Rich export third-party notices',
    '',
    'Generated by node scripts/build-rich-export.mjs from the installed, lockfile-pinned packages.',
    'Package attribution comes from module IDs reported by the IIFE bundler, not the entire dependency tree.',
    'The package LICENSE/NOTICE texts are copied verbatim. No package source or private note is included.',
    'KaTeX code uses MIT; its fonts separately use OFL-1.1. Font copyright and Reserved Font Names',
    'are reproduced from all shipped TTF name tables in katex-fonts/font-notices.json.',
    'The standard OFL text is from https://openfontlicense.org/documents/OFL.txt.',
    '',
    'Only the dependencies listed below are covered by this generated bundle. Other application',
    'dependencies, including unused Markdown/YAML/HTML parsers, require their own inventory.',
    '',
    ...dependencies.map((item) => '- ' + item.name + '@' + item.version + ': ' + JSON.stringify(item.license)),
    '',
    'Input hashes and output hashes are in crates/pkms-core/resources/rich-export.manifest.json',
    'and assets/rich-export/manifest.json inside the export resource ZIP.',
    '',
  ].join('\n'));

  for (const input of sourceInputs) {
    if (digest(readFileSync(path.join(root, input.path))) !== input.sha256)
      throw new Error('Build input changed while bundling; rerun: ' + input.path);
  }
  const outputFiles = filesUnder(staged).map((filename) => fileRecord(filename, prefix + posix(path.relative(staged, filename))));
  const manifest = {
    schemaVersion: 1,
    artifact: 'pkms-rich-export',
    rebuild: 'node scripts/build-rich-export.mjs',
    contract: {
      math: '[data-pkms-math="inline|block"] textContent is exact TeX source',
      mermaid: 'pre[data-pkms-mermaid] textContent is exact Mermaid source',
      script: 'assets/rich-export/renderer.js (classic defer, not module)',
      stylesheet: 'assets/rich-export/style.css',
      csp: "default-src 'none'; img-src 'self'; script-src 'self'; font-src 'self' data:; style-src 'self' 'unsafe-inline'",
      fonts: 'KaTeX CSS fallback fonts embedded as data URLs; unmodified font bytes',
      failure: 'visible reason and original source; successful source remains in expandable details',
      safety: 'shared math-render.ts, mermaid-render.ts, render-policy.ts; no source callback binding',
      archiveCompression: 'Stored; compatible with the standalone core ZIP feature set',
    },
    toolchain: {
      node: process.versions.node,
      vite: packageFor(requireDesktop.resolve('vite')).metadata.version,
      rolldown: packageFor(requireVite.resolve('rolldown')).metadata.version,
      singleBundle: 'IIFE, codeSplitting=false (inlineDynamicImports equivalent)',
    },
    inputFiles: sourceInputs,
    bundledModuleInputs: moduleInputs,
    dependencies,
    fonts: { license: 'OFL-1.1', files: fonts, noticeFiles: ['licenses/katex-fonts/OFL.txt', 'licenses/katex-fonts/font-notices.json'] },
    files: outputFiles,
  };
  const manifestBytes = Buffer.from(JSON.stringify(manifest, null, 2) + '\n');
  writeStaged('manifest.json', manifestBytes);
  const python = process.env.PKMS_PYTHON ?? 'python';
  const zipCode = [
    'from pathlib import Path',
    'import sys, zipfile',
    'source, output = Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve()',
    'with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_STORED) as archive:',
    '    for filename in sorted(source.rglob("*")):',
    '        if filename.is_symlink(): raise RuntimeError("Symlink in resource bundle")',
    '        if not filename.is_file(): continue',
    '        relative = filename.relative_to(source).as_posix()',
    '        info = zipfile.ZipInfo("assets/rich-export/" + relative, (2020, 1, 1, 0, 0, 0))',
    '        info.create_system = 3',
    '        info.external_attr = 0o100644 << 16',
    '        archive.writestr(info, filename.read_bytes(), compress_type=zipfile.ZIP_STORED)',
  ].join('\n');
  const zipped = spawnSync(python, ['-c', zipCode, staged, pendingArchive], { stdio: 'inherit', windowsHide: true });
  if (zipped.error) throw zipped.error;
  if (zipped.status !== 0) throw new Error('Resource ZIP generation failed: ' + zipped.status);
  const archive = fileRecord(pendingArchive, 'crates/pkms-core/resources/rich-export.zip');
  writeFileSync(pendingSidecar, JSON.stringify({
    ...manifest, archive, packagedManifestSha256: digest(manifestBytes),
  }, null, 2) + '\n');
  for (const filename of filesUnder(path.join(staged, 'licenses'))) {
    const destination = path.resolve(installedLicenses, path.relative(path.join(staged, 'licenses'), filename));
    if (!inside(installedLicenses, destination)) throw new Error('Invalid installed license path.');
    writeIfChanged(destination, readFileSync(filename));
  }
  mkdirSync(resources, { recursive: true });
  // The system temp directory may be on another drive. publishIfChanged writes
  // a same-volume temporary file and renames it atomically only when content
  // changed, so unchanged resources keep their mtimes and Cargo can reuse cache.
  writeIfChanged(archivePath, readFileSync(pendingArchive));
  writeIfChanged(sidecarPath, readFileSync(pendingSidecar));
  console.log(JSON.stringify({
    archive, manifest: 'crates/pkms-core/resources/rich-export.manifest.json',
    files: outputFiles.length + 1, dependencies: dependencies.length,
    bundledModules: moduleInputs.length, fontFiles: fonts.length,
  }, null, 2));
} finally {
  const tempParent = realpathSync(tmpdir());
  if (path.dirname(temporary) !== tempParent || !path.basename(temporary).startsWith('pkms-rich-export-'))
    throw new Error('Refusing to remove an unexpected build directory.');
  rmSync(temporary, { recursive: true, force: true });
}
