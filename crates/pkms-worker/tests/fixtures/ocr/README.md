# Synthetic OCR fixture

The PNG and image-only PDF contain three deliberately synthetic lines:

```text
Spanseek OCR 2026
跨索知识管理
离线识别中文资料
```

They contain no user content. `scripts/verify-ocr-runtime.py` runs the actual
packaged Tesseract engine and PDF worker against them, including Unicode paths.
The PDF has no text layer; successful extraction must report the OCR parser.
