"""Reject non-system DLL imports in the statically linked Windows OCR runtime."""
import argparse
import json
import pathlib
import struct


def imports(path):
    data = pathlib.Path(path).read_bytes()
    pe = struct.unpack_from('<I', data, 0x3c)[0]
    if data[pe:pe + 4] != b'PE\0\0':
        raise ValueError('Not a PE image')
    machine, count = struct.unpack_from('<HH', data, pe + 4)
    size = struct.unpack_from('<H', data, pe + 20)[0]
    optional = pe + 24
    if machine != 0x8664 or struct.unpack_from('<H', data, optional)[0] != 0x20b:
        raise ValueError('OCR runtime must be Windows x64 PE32+')
    sections = []
    for index in range(count):
        offset = optional + size + index * 40
        virtual_size, rva, raw_size, raw = struct.unpack_from('<IIII', data, offset + 8)
        sections.append((rva, max(virtual_size, raw_size), raw))

    def offset(rva):
        for start, length, raw in sections:
            if start <= rva < start + length:
                return raw + rva - start
        raise ValueError(f'Invalid PE RVA: {rva}')

    def string(rva):
        start = offset(rva)
        return data[start:data.index(b'\0', start)].decode('ascii').lower()

    names = set()
    directory = optional + 112
    for index, stride, name_index in ((1, 20, 3), (13, 32, 1)):
        rva, length = struct.unpack_from('<II', data, directory + index * 8)
        if not rva:
            continue
        start = offset(rva)
        for position in range(start, start + length, stride):
            values = struct.unpack_from('<' + 'I' * (stride // 4), data, position)
            if not any(values):
                break
            if index == 13 and values[0] != 1:
                raise ValueError('Unsupported delay import address mode')
            names.add(string(values[name_index]))
    return sorted(names)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('executable', type=pathlib.Path)
    args = parser.parse_args()
    names = imports(args.executable)
    allowed = {'kernel32.dll', 'advapi32.dll', 'user32.dll', 'shell32.dll',
               'ole32.dll', 'oleaut32.dll', 'ws2_32.dll', 'bcrypt.dll',
               'ntdll.dll', 'gdi32.dll', 'secur32.dll', 'shlwapi.dll'}
    unexpected = [name for name in names if name not in allowed]
    print(json.dumps({'imports': names, 'unexpected': unexpected}))
    if unexpected:
        raise SystemExit('OCR runtime has unreviewed non-system dependencies')


if __name__ == '__main__':
    main()
