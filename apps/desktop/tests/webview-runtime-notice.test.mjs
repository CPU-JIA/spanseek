import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import {
  WEBVIEW2_INSTALL_NOTICE,
  WEBVIEW2_THRESHOLDS,
  compareWebViewVersions,
  webView2Compatibility,
  webView2Notice,
  webView2VersionFromUserAgent,
} from '../src/diagnostics/webview-compat.ts';

/**
 * An old WebView2 starts the app and silently drops style declarations. The
 * diagnostics page already names that, but only a reader who goes there sees it,
 * so the shell says it once on the way in — from the version the running WebView
 * reports about itself.
 */
const userAgent = (edge) =>
  `Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/${edge} Safari/537.36 Edg/${edge}`;

test('the runtime version is read from the user agent the WebView reports', () => {
  assert.equal(webView2VersionFromUserAgent(userAgent('111.0.1661.44')), '111.0.1661.44');
});

test('a user agent without the Edge token reports no version instead of guessing', () => {
  assert.equal(webView2VersionFromUserAgent('Mozilla/5.0 (X11; Linux x86_64) Firefox/128.0'), null);
  assert.equal(webView2VersionFromUserAgent(''), null);
});

test('the thresholds are newest first, so the missing head names the version to reach', () => {
  const versions = WEBVIEW2_THRESHOLDS.map((threshold) => threshold.version);
  for (let index = 1; index < versions.length; index += 1) {
    assert.equal(
      compareWebViewVersions(versions[index - 1], versions[index]),
      1,
      `${versions[index - 1]} must be newer than ${versions[index]}`,
    );
  }
  const compatibility = webView2Compatibility('105.0.1661.44');
  assert.equal(compatibility.state, 'outdated');
  assert.equal(compatibility.missing[0].version, '123.0.0.0');
});

test('an outdated runtime warns with the version to update to', () => {
  const notice = webView2Notice(webView2Compatibility('105.0.1661.44'));
  assert.equal(notice.level, 'warn');
  assert.match(notice.text, /105\.0\.1661\.44/);
  assert.match(notice.text, /123\.0\.0\.0/);
  assert.match(notice.text, /WebView2 Runtime/);
  assert.match(notice.text, /离线机器/);
  assert.match(
    webView2Notice(webView2Compatibility('105.0.1661.44'), 'en').text,
    /offline machine/,
  );
});

test('a supported runtime and an unknown one stay silent in the shell', () => {
  assert.equal(webView2Compatibility('130.0.0.0').state, 'supported');
  assert.equal(webView2Notice(webView2Compatibility('130.0.0.0')).level, 'info');
  assert.equal(webView2Compatibility(null).state, 'unknown');
  assert.equal(webView2Notice(webView2Compatibility(null)).level, 'info');
});

test('the shell warns once, dismissibly, without replacing the diagnostics panel', async () => {
  const app = await readFile(new URL('../src/App.tsx', import.meta.url), 'utf8');
  assert.match(app, /webView2VersionFromUserAgent\(/);
  assert.match(app, /runtimeNotice\.level === 'warn'/);
  assert.match(app, /setRuntimeNoticeClosed\(true\)/);
  const panel = await readFile(
    new URL('../src/diagnostics/RuntimeEnvironment.tsx', import.meta.url),
    'utf8',
  );
  assert.match(
    panel,
    /diagnosticEnvironment\(versions, language\)/,
    'the panel keeps its own read',
  );
});

test('the installer keeps no invented minimum and states the offline runtime requirement', async () => {
  const config = JSON.parse(
    await readFile(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'),
  );
  assert.equal(config.bundle.windows.nsis.minimumWebView2Version, undefined);
  assert.match(WEBVIEW2_INSTALL_NOTICE[0], /离线/);
  assert.match(WEBVIEW2_INSTALL_NOTICE[0], /WebView2 Runtime/);
  assert.match(WEBVIEW2_INSTALL_NOTICE[1], /offline/i);
});
