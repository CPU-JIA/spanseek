"""Restore the pinned ONNX binaries before reusing Cargo's external link records."""
import argparse
import hashlib
import json
import lzma
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tomllib
import urllib.request

ROOT = Path(__file__).resolve().parent.parent


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def valid(path, entry):
    return path.is_file() and path.stat().st_size == entry['bytes'] and digest(path) == entry['sha256']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cache-root', type=Path)
    parser.add_argument('--downloads', type=Path, default=ROOT / '.local-archive/dependencies/downloads')
    parser.add_argument('--repair-cargo-cache', action='store_true')
    args = parser.parse_args()
    manifest = json.loads((ROOT / 'third-party/native-notices/onnxruntime-1.28.0/runtime-manifest.json').read_text())
    lock = tomllib.loads((ROOT / 'Cargo.lock').read_text(encoding='utf-8'))
    if not any(p['name'] == 'ort-sys' and p['version'] == manifest['ort_sys'] for p in lock['package']):
        raise ValueError('Review the ONNX runtime pin after updating ort-sys')
    if os.environ.get('ORT_LIB_PATH') or os.environ.get('ORT_LIB_LOCATION'):
        raise ValueError('The pinned build cannot use a custom ORT library override')
    cache = args.cache_root or Path(os.environ.get('ORT_CACHE_DIR') or (Path(os.environ['LOCALAPPDATA']) / 'ort.pyke.io'))
    target = cache.resolve() / 'dfbin' / manifest['target'] / manifest['sha256']
    expected = {item['name']: item for item in manifest['files']}
    restored = not all(valid(target / name, entry) for name, entry in expected.items())
    if restored:
        args.downloads.mkdir(parents=True, exist_ok=True)
        archive = args.downloads / f"onnxruntime-{manifest['version']}-{manifest['sha256'][:12]}.tar.lzma2"
        if not archive.exists():
            pending = archive.with_suffix('.download')
            with urllib.request.urlopen(manifest['url'], timeout=120) as response, pending.open('wb') as stream:
                shutil.copyfileobj(response, stream)
            if digest(pending) != manifest['sha256']:
                raise ValueError('Downloaded ONNX archive checksum mismatch')
            pending.replace(archive)
        if digest(archive) != manifest['sha256']:
            raise ValueError('Cached ONNX archive checksum mismatch')
        target.mkdir(parents=True, exist_ok=True)
        found = set()
        with lzma.open(archive, 'rb', format=lzma.FORMAT_RAW,
                       filters=[{'id': lzma.FILTER_LZMA2, 'dict_size': 1 << 26}]) as decoded:
            with tarfile.open(fileobj=decoded, mode='r|') as tar:
                for member in tar:
                    if member.isdir():
                        continue
                    if not member.isfile() or member.name not in expected or member.name in found:
                        raise ValueError('Unexpected entry in pinned ONNX archive')
                    entry = expected[member.name]
                    if member.size != entry['bytes']:
                        raise ValueError('ONNX archive member size mismatch')
                    pending = target / (member.name + '.partial')
                    with tar.extractfile(member) as source, pending.open('wb') as stream:
                        shutil.copyfileobj(source, stream)
                    if not valid(pending, entry):
                        raise ValueError('Extracted ONNX binary checksum mismatch')
                    pending.replace(target / member.name)
                    found.add(member.name)
        if found != expected.keys():
            raise ValueError('ONNX archive omitted a required native file')
    if not all(valid(target / name, entry) for name, entry in expected.items()):
        raise ValueError('ONNX cache verification failed')
    repaired = False
    if args.repair_cargo_cache:
        metadata = json.loads(subprocess.check_output(
            ['cargo', 'metadata', '--no-deps', '--locked', '--format-version', '1'], cwd=ROOT))
        cargo_target = Path(metadata['target_directory']).resolve()
        if cargo_target == Path(cargo_target.anchor):
            raise ValueError('Refusing a Cargo target at a drive root')
        # ort-sys does not register ORT_CACHE_DIR with rerun-if-env-changed.
        # A restored build-script output can therefore reference another runner's
        # missing cache. Reset only that crate when its link path differs.
        for profile in ('debug', 'release'):
            for output in (cargo_target / profile / 'build').glob('ort-sys-*/output'):
                text = output.read_text(encoding='utf-8')
                if 'cargo:rustc-cfg=pyke' not in text:
                    continue
                paths = [line.split('=', 2)[-1] for line in text.splitlines()
                         if line.startswith('cargo:rustc-link-search=native=')]
                if not any(Path(path).resolve() == target for path in paths):
                    repaired = True
        if repaired:
            subprocess.run(['cargo', 'clean', '-p', 'ort-sys', '--target-dir', str(cargo_target)],
                           cwd=ROOT, check=True)
    print(json.dumps({'version': manifest['version'], 'archive_sha256': manifest['sha256'],
                      'native_files_verified': len(expected), 'restored': restored,
                      'stale_cargo_link_records_reset': repaired}))


if __name__ == '__main__':
    main()
