"""Collect complete upstream license/notice text and required component sources.

The inventory deliberately includes resolved build/dev dependencies as well as
runtime dependencies. A package declaration alone never counts as license text.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tomllib
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parent.parent
LICENSE_NAME = re.compile(r'(?i)(^licen[cs]e|^copying|^notice|^copyright|^unlicense|^patents|^ofl[.]|third.?party.*notice)')


def command(*args):
    return subprocess.check_output(args, cwd=ROOT, encoding='utf-8')


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')


def license_files(directory):
    found = []
    for base, dirs, files in os.walk(directory):
        dirs[:] = sorted(d for d in dirs if d not in {
            'node_modules', '.git', 'target', 'tests', 'test', 'benches', 'examples'
        } and not Path(base, d).is_symlink())
        for name in sorted(files):
            if LICENSE_NAME.search(name):
                path = Path(base, name)
                if path.is_file():
                    found.append(path)
    return found


def download(url, path, expected):
    path.parent.mkdir(parents=True, exist_ok=True)
    if not path.exists():
        pending = path.with_suffix(path.suffix + '.partial')
        with urllib.request.urlopen(url, timeout=120) as response, pending.open('wb') as stream:
            while chunk := response.read(1024 * 1024):
                stream.write(chunk)
        if sha(pending) != expected.lower():
            raise ValueError(f'Source archive checksum mismatch: {path.name}')
        pending.replace(path)
    if sha(path) != expected.lower():
        raise ValueError(f'Cached source archive checksum mismatch: {path.name}')
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT / '.local-archive/license-bundle')
    parser.add_argument('--cache', type=Path, default=ROOT / '.local-archive/dependencies/license-sources')
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    cargo = json.loads(command('cargo', 'metadata', '--locked', '--all-features',
                               '--format-version', '1', '--filter-platform', 'x86_64-pc-windows-msvc'))
    npm = json.loads(command('pnpm.cmd' if os.name == 'nt' else 'pnpm', 'licenses', 'list', '--json'))
    supplements = json.loads((ROOT / 'third-party/license-supplements/manifest.json').read_text(encoding='utf-8'))
    extra = {item['id']: item for item in supplements['packages']}
    resolutions = supplements.get('license_resolutions', {})
    packages = {}
    for package in cargo['packages']:
        if package['id'] in cargo['workspace_members']:
            continue
        key = f"cargo:{package['name']}@{package['version']}"
        packages[key] = {'name': package['name'], 'version': package['version'],
                         'license': package.get('license'),
                         'directory': Path(package['manifest_path']).parent,
                         'repository': package.get('repository')}
    for group in npm.values():
        for package in group:
            for path in package['paths']:
                directory = Path(path)
                manifest = json.loads((directory / 'package.json').read_text(encoding='utf-8'))
                key = f"npm:{manifest['name']}@{manifest['version']}"
                packages[key] = {'name': manifest['name'], 'version': manifest['version'],
                                 'license': package.get('license'), 'directory': directory,
                                 'repository': manifest.get('repository')}
    ort = packages.get('cargo:ort-sys@2.0.0-rc.13')
    native_origin_path = ROOT / 'third-party/native-notices/onnxruntime-1.28.0/ORIGIN.json'
    native_origin = json.loads(native_origin_path.read_text(encoding='utf-8'))
    if not ort or native_origin['archive_sha256'] not in (ort['directory'] / 'build/download/dist.tsv').read_text(encoding='utf-8'):
        raise ValueError('Review ONNX Runtime notices/source versions after changing ort-sys')
    for file in native_origin['files']:
        if sha(native_origin_path.parent / file['path']) != file['sha256']:
            raise ValueError('ONNX Runtime notice text changed without updating its origin')
    text = ['spanseek — Complete third-party license and notice texts',
            'Scope: resolved Cargo/installed pnpm dependencies (including build/dev tools),',
            'plus bundled assets and native runtimes. This is not a binary composition claim.',
            'Third-party components retain their own licenses; the spanseek proprietary',
            'license does not restrict those rights. Sources: THIRD-PARTY-SOURCES.zip.', '']
    records = []
    missing = []

    def append(identity, path, label):
        data = path.read_bytes()
        try:
            content = data.decode('utf-8-sig')
        except UnicodeDecodeError:
            content = data.decode('cp1252')
        if '\x00' in content:
            raise ValueError(f'License must be text: {identity}/{label}')
        text.extend(['=' * 78, identity + ' / ' + label, '-' * 78, content, ''])
        return {'file': label, 'sha256': hashlib.sha256(data).hexdigest()}

    for key, package in sorted(packages.items()):
        directory = package['directory']
        license_name = package['license']
        if key in resolutions:
            resolution = resolutions[key]
            if sha(directory / resolution['evidence_file']) != resolution['sha256']:
                raise ValueError(f'License resolution evidence changed: {key}')
            license_name = resolution['license']
        files = []
        for path in license_files(directory):
            files.append(append(key, path, path.relative_to(directory).as_posix()))
        supplement = extra.get(key)
        if supplement:
            for file in supplement['files']:
                path = ROOT / file['path']
                if sha(path) != file['sha256']:
                    raise ValueError(f'License supplement checksum mismatch: {key}')
                files.append(append(key, path, 'supplement/' + path.name))
        if not files or not license_name or license_name.lower() in ('unknown', 'undeclared'):
            missing.append(key)
        if license_name and ('GPL' in license_name or 'EPL' in license_name):
            if key != 'npm:elkjs@0.9.3' and not (' OR ' in license_name and ('MIT' in license_name or 'Apache-2.0' in license_name)):
                raise ValueError(f'Review corresponding source obligations for {key}: {license_name}')
        records.append({'id': key, 'license': license_name, 'repository': package['repository'],
                        'files': files, 'supplement': supplement})
    asset_roots = [ROOT / 'third-party', ROOT / '.local-archive/dependencies/pdfium-7881',
                   ROOT / '.local-archive/dependencies/tesseract-5.5.4']
    for directory in asset_roots:
        if not directory.is_dir():
            raise ValueError(f'Native or asset directory missing: {directory.name}')
        paths = license_files(directory)
        if directory.name == 'pdfium-7881':
            paths = sorted(set(paths) | {p for p in (directory / 'licenses').rglob('*') if p.is_file()})
        for path in paths:
            # Package supplements were already included above.
            if 'license-supplements' in path.parts:
                continue
            identity = 'asset:' + directory.name
            records.append({'id': identity, 'files': [append(identity, path, path.relative_to(directory).as_posix())]})
    write_json(output / 'THIRD-PARTY-COMPONENTS.json', {'scope': text[1:4], 'packages': records, 'missing': missing})
    if missing:
        raise SystemExit('Missing complete license evidence: ' + ', '.join(missing))
    (output / 'THIRD-PARTY-LICENSES.txt').write_text('\n'.join(text), encoding='utf-8')

    source_manifest = json.loads((ROOT / 'third-party/copyleft-sources.json').read_text(encoding='utf-8'))
    lock = tomllib.loads((ROOT / 'Cargo.lock').read_text(encoding='utf-8'))
    sources = []
    for package in source_manifest['cargo']:
        name, version = package['name'], package['version']
        key = f'cargo:{name}@{version}'
        if key not in packages:
            raise ValueError(f'Source manifest no longer matches dependencies: {key}')
        locked = next(p for p in lock['package'] if p['name'] == name and p['version'] == version)
        filename = f'{name}-{version}.crate'
        url = f'https://static.crates.io/crates/{name}/{filename}'
        path = download(url, args.cache / filename, locked['checksum'])
        sources.append((path, 'cargo/' + filename, url))
    expected_copyleft = {key for key, package in packages.items() if key.startswith('cargo:')
                         and package['license'] == 'MPL-2.0'}
    covered = {f"cargo:{p['name']}@{p['version']}" for p in source_manifest['cargo']}
    if expected_copyleft != covered:
        raise ValueError('Update the source manifest for changed MPL dependencies')
    for entry in source_manifest['downloads']:
        path = download(entry['url'], args.cache / entry['file'], entry['sha256'])
        sources.append((path, entry.get('subdirectory', 'elk') + '/' + entry['file'], entry['url']))
    for filename in source_manifest['local_files']:
        folder = 'elk' if '/elkjs/' in filename else 'onnxruntime-eigen'
        sources.append((ROOT / filename, folder + '/' + Path(filename).name, 'repository:' + filename))
    source_index = [{'file': name, 'url': url, 'sha256': sha(path)} for path, name, url in sources]
    instructions = (
        'Source code for third-party components\n\n'
        'Rust .crate files are unmodified tar.gz archives from crates.io, verified\n'
        'against Cargo.lock. They include source/build files under MPL-2.0.\n'
        'ELK Java and elkjs JavaScript sources, plus the upstream patch used by\n'
        'elkjs 0.9.3, are in elk/. See elk/README.md for the build instructions.\n'
        'Eigen source used by ONNX Runtime 1.28.0 is in onnxruntime-eigen/;\n'
        'the source revision and upstream SHA-1 are fixed by the included deps.txt.\n'
        'No spanseek changes were made to these components. All original license\n'
        'and copyright notices remain. Their licenses allow recipients to obtain,\n'
        'modify and redistribute the covered source under those terms.\n'
        'lightningcss is CSS build tooling and is not copied into the application.\n'
        'DOMPurify uses its Apache-2.0 option; JSZip uses its MIT option.\n'
    )
    (output / 'THIRD-PARTY-SOURCE-INFO.txt').write_text(instructions, encoding='utf-8')
    with zipfile.ZipFile(output / 'THIRD-PARTY-SOURCES.zip', 'w', zipfile.ZIP_DEFLATED) as archive:
        archive.writestr('README.txt', instructions)
        archive.writestr('SOURCES.json', json.dumps(source_index, indent=2) + '\n')
        for path, name, _ in sources:
            archive.write(path, name)
    print(json.dumps({'packages': len(packages), 'license_records': len(records),
                      'missing': len(missing), 'source_archives': len(sources),
                      'license_bytes': (output / 'THIRD-PARTY-LICENSES.txt').stat().st_size}))


if __name__ == '__main__':
    main()
