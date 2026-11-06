"""Reject non-system DLL imports in the statically linked Windows OCR runtime."""
import argparse
import json
import pathlib
import struct
import ctypes
import xml.etree.ElementTree as ET


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
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    kernel.LoadLibraryExW.argtypes = [ctypes.c_wchar_p, ctypes.c_void_p, ctypes.c_uint32]
    kernel.LoadLibraryExW.restype = ctypes.c_void_p
    kernel.FindResourceW.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p]
    kernel.FindResourceW.restype = ctypes.c_void_p
    kernel.LoadResource.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    kernel.LoadResource.restype = ctypes.c_void_p
    kernel.LockResource.argtypes = [ctypes.c_void_p]
    kernel.LockResource.restype = ctypes.c_void_p
    kernel.SizeofResource.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    kernel.SizeofResource.restype = ctypes.c_uint32
    kernel.FreeLibrary.argtypes = [ctypes.c_void_p]
    module = kernel.LoadLibraryExW(str(args.executable.resolve()), None, 0x22)
    if not module:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        resource = kernel.FindResourceW(module, 1, 24)
        if not resource:
            raise ValueError('OCR executable has no embedded application manifest')
        size = kernel.SizeofResource(module, resource)
        pointer = kernel.LockResource(kernel.LoadResource(module, resource))
        manifest = ET.fromstring(ctypes.string_at(pointer, size))
        node = manifest.find('.//{http://schemas.microsoft.com/SMI/2019/WindowsSettings}activeCodePage')
        codepage = node.text if node is not None else None
    finally:
        kernel.FreeLibrary(module)
    if codepage != 'UTF-8':
        raise ValueError('OCR process must use UTF-8 independently of the Windows system locale')
    print(json.dumps({'imports': names, 'unexpected': unexpected, 'active_code_page': codepage}))
    if unexpected:
        raise SystemExit('OCR runtime has unreviewed non-system dependencies')


if __name__ == '__main__':
    main()
