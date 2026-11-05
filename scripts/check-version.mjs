import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

const root = fileURLToPath(new URL('../', import.meta.url));
const readJson = (path) => JSON.parse(readFileSync(join(root, path), 'utf8'));
const cargo = readFileSync(join(root, 'Cargo.toml'), 'utf8');
const version = cargo.match(/\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)?.[1];
if (!version || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
  throw new Error('Cargo workspace version is missing or invalid');
}
for (const file of ['apps/desktop/package.json', 'apps/desktop/src-tauri/tauri.conf.json']) {
  if (readJson(file).version !== version)
    throw new Error(`Version differs from Cargo workspace: ${file}`);
}
const tagIndex = process.argv.indexOf('--tag');
if (tagIndex !== -1 && process.argv[tagIndex + 1] !== `v${version}`) {
  throw new Error(`Release tag must be v${version}`);
}
console.log(version);
