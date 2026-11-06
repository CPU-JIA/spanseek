"""Exercise the real packaged engine and PDF worker on synthetic bilingual input."""
import argparse
import hashlib
import json
import pathlib
import shutil
import subprocess
import uuid

ROOT = pathlib.Path(__file__).resolve().parent.parent


def require_text(text):
    compact = ''.join(text.split())
    for expected in ('SpanseekOCR2026', '跨索知识管理', '离线识别中文资料'):
        if expected not in compact:
            raise AssertionError(f'OCR did not recognize expected fixture text: {expected}; got {text!r}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runtime', type=pathlib.Path, required=True)
    parser.add_argument('--worker', type=pathlib.Path, required=True)
    parser.add_argument('--pdfium', type=pathlib.Path, required=True)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    runtime = output / '引擎 中文路径'
    shutil.copytree(args.runtime.resolve(), runtime)
    fixture = ROOT / 'crates/pkms-worker/tests/fixtures/ocr'
    image = output / '中英文扫描 图片.png'
    shutil.copyfile(fixture / 'bilingual.png', image)
    engine = runtime / 'tesseract.exe'
    command = [str(engine), str(image), 'stdout', '-l', 'chi_sim+eng',
               '--tessdata-dir', str(runtime / 'tessdata'), '--psm', '3']
    direct = subprocess.run(command, capture_output=True, timeout=60, check=True)
    text = direct.stdout.decode('utf-8')
    require_text(text)
    work = output / '扫描 PDF 输出'
    work.mkdir()
    pdf = output / '无文本层 中英文.pdf'
    shutil.copyfile(fixture / 'bilingual-scan.pdf', pdf)
    request = {
        'protocol_version': 1, 'job_id': str(uuid.uuid4()), 'attempt': 1,
        'lease_token': str(uuid.uuid4()), 'source_version_id': str(uuid.uuid4()),
        'input_path': str(pdf), 'format': 'pdf', 'output_dir': str(work),
        'pdfium_library_path': str(args.pdfium.resolve()), 'content_type': 'application/pdf',
        'ocr': {'executable': str(engine), 'tessdata_dir': str(runtime / 'tessdata'),
                'languages': 'chi_sim+eng', 'dpi': 300},
    }
    response = subprocess.run([str(args.worker.resolve())], input=json.dumps(request).encode(),
                              capture_output=True, timeout=90, check=True)
    result = json.loads(response.stdout)
    if result['result']['status'] != 'success':
        raise AssertionError(f'PDF worker failed: {result}')
    parsed_file = work / result['result']['result_file']
    if hashlib.sha256(parsed_file.read_bytes()).hexdigest() != result['result']['sha256']:
        raise AssertionError('Worker response hash differs from parsed output')
    parsed = json.loads(parsed_file.read_text(encoding='utf-8'))
    pdf_text = '\n'.join(block['text'] for block in parsed['blocks'])
    require_text(pdf_text)
    if parsed.get('needs_ocr') or '+ocr-' not in parsed.get('parser_version', ''):
        raise AssertionError('PDF must report OCR applied and no remaining OCR requirement')
    record = {'engine_sha256': hashlib.sha256(engine.read_bytes()).hexdigest(),
              'png_text': text, 'pdf_text': pdf_text, 'worker_result': result['result'],
              'unicode_paths': True, 'passed': True}
    (output / 'verification.json').write_text(json.dumps(record, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(record, ensure_ascii=False))


if __name__ == '__main__':
    main()
