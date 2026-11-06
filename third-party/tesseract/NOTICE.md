# Tesseract source-build notice

- Engine: Tesseract 5.5.4, Apache-2.0; upstream AUTHORS and LICENSE are preserved.
- Recipe: `third-party/tesseract/build/vcpkg.json`, pinned vcpkg baseline and source hashes.
- Build: `scripts/build-tesseract.ps1`; MSVC, Windows x64, static CRT and libraries.
- The three vcpkg Tesseract patches are included with the installed port recipe.
- No spanseek changes to OCR algorithms; network/archive/training features are disabled.
- The executable's Windows manifest is merged with `build/utf8.manifest` so its
  narrow argv/file APIs use UTF-8 on Windows 10 1903 and later, independently of
  the system language. The existing UAC execution level is preserved. This is
  a packaging change; the vcpkg source and OCR algorithms remain unchanged.
- Language data: chi_sim and eng from tessdata_fast, Apache-2.0, independently SHA-256 pinned.
- Installation: `scripts/prepare-tesseract.ps1` verifies the build receipt, copies
  the executable, notices and languages, and checks PE imports/version/languages.
- `COMPONENTS.md` is an overview. Complete upstream license text and exact build
  recipes are installed in `tesseract/licenses/`; metadata is not a substitute.
