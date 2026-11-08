"""Release-metadata and rendered-download regression tests; no network writes."""
import copy
import hashlib
import json
import os
import tempfile
from pathlib import Path
import unittest
from release_sync import API, DOWNLOADS, MetadataFetchError, render, synchronize, validate_release


def fixture():
    version = "9.8.7"
    name = f"spanseek_{version}_x64-setup.exe"
    info = {"version": version, "installer": name, "platform": "windows-x64", "bytes": 12345678,
            "sha256": "a" * 64, "signature": "NotSigned", "built_at": "2026-11-08T10:00:00Z",
            "release_state": "draft"}  # Build-time state is not the publication state.
    body = json.dumps(info).encode()
    sums = (hashlib.sha256(body).hexdigest() + "  BUILD-INFO.json\n" + info["sha256"] + "  " + name + "\n").encode()
    blobs = {"BUILD-INFO.json": body, "SHA256SUMS.txt": sums}
    assets = [{"id": i, "name": n, "state": "uploaded", "size": len(b), "digest": "sha256:" + hashlib.sha256(b).hexdigest(),
               "browser_download_url": f"{DOWNLOADS}/v{version}/{n}"} for i, (n, b) in enumerate(blobs.items(), start=1)]
    assets.append({"id": 3, "name": name, "state": "uploaded", "size": info["bytes"], "digest": "sha256:" + info["sha256"], "browser_download_url": f"{DOWNLOADS}/v{version}/{name}"})
    release = {"id": 4, "tag_name": "v" + version, "draft": False, "prerelease": False, "published_at": "2026-11-08T11:00:00Z", "assets": assets}
    by_id = {str(asset['id']): blobs[asset['name']] for asset in assets if asset['name'] in blobs}
    return release, lambda url: by_id[url.rsplit("/", 1)[1]]


class ReleaseSyncTests(unittest.TestCase):
    def make_site(self):
        runs = Path(os.environ['SPANSEEK_TEST_ROOT']) if 'SPANSEEK_TEST_ROOT' in os.environ else Path(__file__).resolve().parent.parent / '.local-archive' / 'test-runs' / 'website-release-sync-tests'
        runs.mkdir(parents=True, exist_ok=True)
        root = Path(tempfile.mkdtemp(prefix='sync-', dir=runs))
        base = root / 'releases' / 'base'
        (base / 'public').mkdir(parents=True)
        (base / 'templates').mkdir()
        for name in ('index.html', 'install.html'):
            (base / 'public' / name).write_text('initial website', encoding='utf-8')
            (base / 'templates' / name).write_text('version __RELEASE_VERSION__ hash __RELEASE_SHA256__', encoding='utf-8')
        self.write_manifest(base)
        release, asset_fetch = fixture()
        fetch = lambda url: json.dumps(release).encode() if url == API else asset_fetch(url)
        return root, base, release, fetch

    def write_manifest(self, base):
        files = [{'path': p.name, 'bytes': p.stat().st_size, 'sha256': hashlib.sha256(p.read_bytes()).hexdigest()} for p in sorted((base / 'public').iterdir())]
        (base / 'public-manifest.json').write_text(json.dumps({'files': files}), encoding='utf-8')

    def test_future_release_updates_all_download_fields(self):
        release, fetch = fixture()
        metadata = validate_release(release, fetch)
        template = '<a href="https://github.com/CPU-JIA/spanseek/releases/download/v__RELEASE_VERSION__/spanseek___RELEASE_VERSION___x64-setup.exe">__RELEASE_VERSION__</a> __RELEASE_SIZE__ __RELEASE_DATE__ __RELEASE_SHA256__ __RELEASE_SIGNATURE__'
        html = render(template, metadata)
        self.assertIn('/v9.8.7/spanseek_9.8.7_x64-setup.exe', html)
        self.assertIn('11.77 MiB', html)
        self.assertIn('2026-11-08', html)
        self.assertIn('A' * 64, html)
        self.assertNotIn('__RELEASE_', html)

    def test_drafts_and_previews_cannot_replace_a_stable_release(self):
        release, fetch = fixture()
        for key in ('draft', 'prerelease'):
            changed = copy.deepcopy(release)
            changed[key] = True
            with self.subTest(key=key), self.assertRaises(ValueError):
                validate_release(changed, fetch)

    def test_partial_upload_is_rejected(self):
        release, fetch = fixture()
        release['assets'].pop()
        with self.assertRaises(ValueError):
            validate_release(release, fetch)

    def test_download_tampering_is_rejected(self):
        release, fetch = fixture()
        with self.assertRaises(ValueError):
            validate_release(release, lambda url: fetch(url) + b'corrupt')

    def test_inconsistent_installer_digest_is_rejected(self):
        release, fetch = fixture()
        release['assets'][-1]['digest'] = 'sha256:' + 'b' * 64
        with self.assertRaises(ValueError):
            validate_release(release, fetch)

    def test_external_download_url_is_rejected(self):
        release, fetch = fixture()
        release['assets'][-1]['browser_download_url'] = 'https://example.com/setup.exe'
        with self.assertRaises(ValueError):
            validate_release(release, fetch)

    def test_transient_asset_gateway_failure_uses_the_same_verified_asset(self):
        release, original_fetch = fixture()
        by_name = {a['name']: a['id'] for a in release['assets']}
        def fetch(url):
            if '/releases/assets/' in url:
                raise MetadataFetchError('fixture gateway failure', True)
            return original_fetch('https://api.github.com/assets/' + str(by_name[url.rsplit('/', 1)[1]]))
        self.assertEqual(validate_release(release, fetch)['version'], '9.8.7')

    @unittest.skipIf(os.name == 'nt', 'Atomic symlink deployment requires the target Linux environment')
    def test_atomic_update_idempotence_and_failed_metadata_preserve_current(self):
        root, base, release, fetch = self.make_site()
        self.assertEqual(synchronize(root, base, fetch)['status'], 'updated')
        current = (root / 'current').resolve()
        self.assertIn('9.8.7', (current / 'index.html').read_text())
        self.assertEqual(synchronize(root, base, fetch)['status'], 'unchanged')
        release['assets'][-1]['digest'] = 'sha256:' + 'b' * 64
        with self.assertRaises(ValueError):
            synchronize(root, base, fetch)
        self.assertEqual((root / 'current').resolve(), current)

    @unittest.skipIf(os.name == 'nt', 'Atomic symlink deployment requires the target Linux environment')
    def test_new_site_assets_trigger_update_even_when_html_and_release_are_unchanged(self):
        root, base, release, fetch = self.make_site()
        synchronize(root, base, fetch)
        old_current = (root / 'current').resolve()
        (base / 'public' / 'styles.css').write_text('new approved site styles', encoding='utf-8')
        self.write_manifest(base)
        self.assertEqual(synchronize(root, base, fetch)['status'], 'updated')
        self.assertNotEqual((root / 'current').resolve(), old_current)
        self.assertEqual((root / 'current' / 'styles.css').read_text(), 'new approved site styles')


if __name__ == '__main__':
    unittest.main()
