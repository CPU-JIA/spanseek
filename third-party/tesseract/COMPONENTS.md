# Bundled Tesseract runtime components

The installer-facing bundle contains an unmodified runtime subset from the
pinned UB Mannheim Tesseract 5.4.0.20240606 Windows x64 package, plus
`chi_sim` and `eng` data from `tesseract-ocr/tessdata_fast` commit
`87416418657359cb625c412a48b6e1d6d41c29bd`.

| Component | Version | License | Upstream |
| --- | --- | --- | --- |
| Tesseract OCR | 5.4.0.20240606 | Apache-2.0 | https://github.com/tesseract-ocr/tesseract |
| Leptonica | 1.84.1 | BSD-2-Clause | https://github.com/DanBloomberg/leptonica |
| libarchive | 3.7.4 | BSD-2-Clause | https://github.com/libarchive/libarchive |
| OpenSSL libcrypto | 3.3.1 | Apache-2.0 | https://github.com/openssl/openssl |
| libjpeg-turbo | 3.0.1 | IJG AND BSD-3-Clause AND Zlib | https://github.com/libjpeg-turbo/libjpeg-turbo |
| libpng | 1.6.43 | libpng-2.0 | https://github.com/pnggroup/libpng |
| libtiff | 4.6.0 | libtiff | https://gitlab.com/libtiff/libtiff |
| zlib | 1.3 | Zlib | https://github.com/madler/zlib |
| libwebp / libsharpyuv | 1.4.0 | BSD-3-Clause | https://github.com/webmproject/libwebp |
| OpenJPEG | 2.5.2 | BSD-2-Clause | https://github.com/uclouvain/openjpeg |
| XZ Utils liblzma | 5.6.2 | 0BSD | https://github.com/tukaani-project/xz |
| bzip2 | 1.0.8 | bzip2-1.0.6 | https://sourceware.org/bzip2/ |
| LZ4 | 1.9.4 | BSD-2-Clause | https://github.com/lz4/lz4 |
| Zstandard | 1.5.6 | BSD-3-Clause OR GPL-2.0 | https://github.com/facebook/zstd |
| Expat | 2.x | MIT | https://github.com/libexpat/libexpat |
| libiconv | 1.17 | LGPL-2.1-or-later | https://www.gnu.org/software/libiconv/ |
| libdeflate | 1.x | MIT | https://github.com/ebiggers/libdeflate |
| libb2 | 0.98.x | CC0-1.0 | https://github.com/BLAKE2/libb2 |
| LERC | 4.x | Apache-2.0 | https://github.com/Esri/lerc |
| JBIG-KIT | 2.1 | GPL-2.0-or-later | https://www.cl.cam.ac.uk/~mgk25/jbigkit/ |
| giflib | 5.2.1 | MIT | https://sourceforge.net/projects/giflib/ |
| GCC runtime | 13.x | GPL-3.0-or-later WITH GCC-exception-3.1 | https://gcc.gnu.org/ |
| MinGW-w64 winpthreads | 1.0 | MIT | https://www.mingw-w64.org/ |

`runtime-manifest.json` is the machine-readable authority for the exact copied
files and SHA-256 values. `LICENSE`, `AUTHORS` and `README.tesseract.md` are the
upstream Tesseract files. No component has been modified by spanseek.

The language data and Tesseract engine are covered by Apache-2.0. The complete
license texts for all bundled runtime libraries must remain in the release
notices bundle; the component table above records the exact upstream projects
and SPDX expressions used for the audit.