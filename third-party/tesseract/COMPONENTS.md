# Bundled OCR runtime components

Tesseract 5.5.4 is built from source with MSVC and static CRT/libraries.
The vcpkg baseline is `dbb76fb224d7696f99d7a33cf35db4a86b1b2f44`.
The application uses local PNG input and pinned `chi_sim` / `eng` language data.
Tesseract curl, libarchive and training tools are disabled.

| Component | Version | Declared license |
| --- | --- | --- |
| zlib | 1.3.2 | Zlib |
| liblzma | 5.8.4 | 0BSD |
| libspng | 0.7.4 | BSD-2-Clause AND libpng-2.0 |
| libjpeg-turbo | 3.2.1 | BSD-3-Clause AND IJG |
| tiff | 4.7.2 | libtiff |
| openjpeg | 2.5.4 | BSD-2-Clause |
| libwebp | 1.6.0 | BSD-3-Clause |
| libpng | 1.6.59 | libpng-2.0 |
| giflib | 6.1.3 | MIT |
| leptonica | 1.87.0 | See the complete upstream copyright text (Leptonica) |
| tesseract | 5.5.4 | Apache-2.0 |

Every native component has its complete vcpkg `copyright` text and pinned port
recipe/patches under the installed `tesseract/licenses/<component>/` directory.
The build receipt records versions, port ABIs, source baseline, PE imports and
per-file SHA-256. Recipe source archive hashes are in the included portfiles.

The PE gate rejects non-system DLL imports. MinGW/GCC, JBIG-KIT and libiconv
from the previous prebuilt runtime are not included in this build.

Language data comes from `tesseract-ocr/tessdata_fast` commit
`87416418657359cb625c412a48b6e1d6d41c29bd` under Apache-2.0.
See `runtime-manifest.json` for exact file hashes.
