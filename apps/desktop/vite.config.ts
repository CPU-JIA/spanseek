import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { readFileSync, readdirSync } from 'node:fs';

const require = createRequire(import.meta.url);
const packageVersion = JSON.parse(readFileSync(new URL('./package.json', import.meta.url), 'utf8')).version as string;
const pdfRoot = dirname(require.resolve('pdfjs-dist/package.json'));
// The parser's browser export uses document.createElement during module initialization.
// Workers have no DOM: use the same package's pure entity-table implementation in both
// development and production, without changing resolution conditions for other packages.
const markdownRequire = createRequire(require.resolve('remark-parse'));
const mdastRequire = createRequire(markdownRequire.resolve('mdast-util-from-markdown'));
const namedEntityDecoder = mdastRequire.resolve('decode-named-character-reference');
// PDF.js resolves CMaps/fonts/WASM by their canonical names; keep these files local.
const pdfAssets = new Map<string, Buffer>();
pdfAssets.set('/pdfjs/LICENSE', readFileSync(join(pdfRoot, 'LICENSE')));
for (const folder of ['cmaps', 'standard_fonts', 'wasm']) {
  for (const name of readdirSync(join(pdfRoot, folder))) {
    pdfAssets.set(`/pdfjs/${folder}/${name}`, readFileSync(join(pdfRoot, folder, name)));
  }
}

export default defineConfig({
  define: {
    __PKMS_APP_VERSION__: JSON.stringify(packageVersion),
  },
  resolve: {
    alias: [{ find: /^decode-named-character-reference$/, replacement: namedEntityDecoder }],
  },
  plugins: [
    react(),
    {
      name: 'pkms-local-pdf-assets',
      configureServer(server) {
        server.middlewares.use((request, response, next) => {
          const path = request.url?.split('?')[0] ?? '';
          const data = pdfAssets.get(path);
          if (!data) return next();
          response.setHeader(
            'Content-Type',
            path.endsWith('.wasm') ? 'application/wasm' : 'application/octet-stream',
          );
          response.end(data);
        });
      },
      generateBundle() {
        for (const [path, data] of pdfAssets)
          this.emitFile({ type: 'asset', fileName: path.slice(1), source: data });
      },
    },
  ],
  clearScreen: false,
  server: { host: '127.0.0.1', port: 5173, strictPort: true },
  build: {
    target: 'es2022',
    // Keep even small fonts on the local origin allowed by the desktop CSP.
    assetsInlineLimit: (filePath) =>
      /\.(?:woff2?|ttf|otf|eot)$/i.test(filePath) ? false : undefined,
  },
});
