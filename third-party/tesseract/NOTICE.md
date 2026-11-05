# Tesseract bundle provenance

- Package: UB Mannheim Tesseract OCR for Windows x64
- Version: 5.4.0.20240606
- Installer: https://github.com/UB-Mannheim/tesseract/releases/download/v5.4.0.20240606/tesseract-ocr-w64-setup-5.4.0.20240606.exe
- Installer SHA-256: `C885FFF6998E0608BA4BB8AB51436E1C6775C2BAFC2559A19B423E18678B60C9`
- Runtime subset: 27 files required by `tesseract.exe`; exact hashes are in `runtime-manifest.json`
- Language data: `chi_sim.traineddata` and `eng.traineddata` from `tesseract-ocr/tessdata_fast` commit `87416418657359cb625c412a48b6e1d6d41c29bd`
- Build integration: `scripts/prepare-tesseract.ps1` verifies every file, copies it to `.local-archive/dependencies/tesseract-5.4.0.20240606`, and runs an isolated `--version` / `--list-langs` smoke test.
- Runtime location in the installed product: `resources/tesseract/`.

The app prefers an explicit `PKMS_TESSERACT_EXE` override, then the bundled
resource directory, then the development bundle, then a legacy system install.
The worker requires both `chi_sim` and `eng` before reporting OCR as available.