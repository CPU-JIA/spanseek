use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
use std::path::Path;

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, ReOpenFile, SetFileAttributesW, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_TEMPORARY,
    FILE_FLAG_DELETE_ON_CLOSE, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, MOVEFILE_REPLACE_EXISTING,
};

use crate::util;

fn to_utf16(s: &Path) -> io::Result<Vec<u16>> {
    let mut original: Vec<u16> = s.as_os_str().encode_wide().collect();
    if original.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path contains NUL",
        ));
    }
    // Rust std's Win32 I/O also accepts this NT namespace prefix, but
    // absolute() would treat it as a rooted ordinary path and rewrite it.
    if original.starts_with(&[92, 63, 63, 92]) {
        original.push(0);
        return Ok(original);
    }
    // Resolve relative paths and ordinary Windows separators/dot segments before
    // adding the verbatim prefix, whose semantics disable Win32 normalization.
    // Unlike canonicalize(), absolute() also accepts a not-yet-created target.
    let absolute = std::path::absolute(s)?;
    let path: Vec<u16> = absolute.as_os_str().encode_wide().collect();
    let verbatim: Vec<u16> = r"\\?\".encode_utf16().collect();
    let device: Vec<u16> = r"\\.\".encode_utf16().collect();
    let mut encoded = if path.starts_with(&verbatim) || path.starts_with(&device) {
        path
    } else if path.starts_with(&[92, 92]) {
        r"\\?\UNC\"
            .encode_utf16()
            .chain(path[2..].iter().copied())
            .collect()
    } else {
        verbatim.into_iter().chain(path).collect()
    };
    encoded.push(0);
    Ok(encoded)
}

fn not_supported<T>(msg: &str) -> io::Result<T> {
    Err(io::Error::new(io::ErrorKind::Other, msg))
}

pub fn create_named(
    path: &Path,
    open_options: &mut OpenOptions,
    permissions: Option<&std::fs::Permissions>,
) -> io::Result<File> {
    if permissions.map_or(false, |p| p.readonly()) {
        return not_supported("changing permissions is not supported on this platform");
    }
    open_options
        .create_new(true)
        .read(true)
        .write(true)
        .custom_flags(FILE_ATTRIBUTE_TEMPORARY)
        .open(path)
}

pub fn create(dir: &Path) -> io::Result<File> {
    util::create_helper(
        dir,
        OsStr::new(".tmp"),
        OsStr::new(""),
        crate::NUM_RAND_CHARS,
        |path| {
            let f = OpenOptions::new()
                .create_new(true)
                .read(true)
                .write(true)
                .share_mode(0)
                .custom_flags(FILE_ATTRIBUTE_TEMPORARY | FILE_FLAG_DELETE_ON_CLOSE)
                .open(path)?;
            // NOTE: in theory, we could delete the file immediately (we open the file in "unix
            // semantics" mode) but this seemed to corrupt something in Windows at scale (see #339).
            // So we just rely on `FILE_FLAG_DELETE_ON_CLOSE`.
            Ok(f)
        },
    )
}

pub fn reopen(file: &File, _path: &Path) -> io::Result<File> {
    let handle = file.as_raw_handle();
    unsafe {
        let handle = ReOpenFile(
            handle as HANDLE,
            FILE_GENERIC_READ | FILE_GENERIC_WRITE,
            FILE_SHARE_DELETE | FILE_SHARE_READ | FILE_SHARE_WRITE,
            0,
        );
        if handle == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            Ok(FromRawHandle::from_raw_handle(handle as RawHandle))
        }
    }
}

pub fn keep(path: &Path) -> io::Result<()> {
    unsafe {
        let path_w = to_utf16(path)?;
        if SetFileAttributesW(path_w.as_ptr(), FILE_ATTRIBUTE_NORMAL) == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

pub fn persist(old_path: &Path, new_path: &Path, overwrite: bool) -> io::Result<()> {
    unsafe {
        let old_path_w = to_utf16(old_path)?;
        let new_path_w = to_utf16(new_path)?;

        // Don't succeed if this fails. We don't want to claim to have successfully persisted a file
        // still marked as temporary because this file won't have the same consistency guarantees.
        if SetFileAttributesW(old_path_w.as_ptr(), FILE_ATTRIBUTE_NORMAL) == 0 {
            return Err(io::Error::last_os_error());
        }

        let mut flags = 0;

        if overwrite {
            flags |= MOVEFILE_REPLACE_EXISTING;
        }

        if MoveFileExW(old_path_w.as_ptr(), new_path_w.as_ptr(), flags) == 0 {
            let e = io::Error::last_os_error();
            // If this fails, the temporary file is now un-hidden and no longer marked temporary
            // (slightly less efficient) but it will still work.
            let _ = SetFileAttributesW(old_path_w.as_ptr(), FILE_ATTRIBUTE_TEMPORARY);
            Err(e)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod long_path_tests {
    use super::*;

    #[test]
    fn nt_namespace_keep_and_persist() {
        let directory = crate::tempdir().unwrap();
        let source = directory.path().join("source.txt");
        let destination = directory.path().join("destination.txt");
        std::fs::write(&source, b"NT namespace payload").unwrap();
        let nt_source = format!(r"\??\{}", source.display());
        let nt_destination = format!(r"\??\{}", destination.display());
        keep(Path::new(&nt_source)).unwrap();
        persist(Path::new(&nt_source), Path::new(&nt_destination), false).unwrap();
        assert_eq!(std::fs::read(destination).unwrap(), b"NT namespace payload");
        assert!(!source.exists());
    }

    fn encoded(path: &str) -> String {
        let wide = to_utf16(Path::new(path)).unwrap();
        assert_eq!(wide.last(), Some(&0));
        String::from_utf16(&wide[..wide.len() - 1]).unwrap()
    }

    #[test]
    fn resolves_absolute_relative_unc_and_verbatim_paths() {
        assert_eq!(encoded(r"C:/folder/../知识/file"), r"\\?\C:\知识\file");
        assert_eq!(
            encoded(r"\\server\share\folder\..\file"),
            r"\\?\UNC\server\share\file"
        );
        assert_eq!(encoded(r"\\?\C:\folder\file"), r"\\?\C:\folder\file");
        assert_eq!(
            encoded(r"\\?\UNC\server\share\file"),
            r"\\?\UNC\server\share\file"
        );
        assert_eq!(encoded(r"\\.\C:\folder\file"), r"\\.\C:\folder\file");
        assert_eq!(encoded(r"\??\C:\folder\file"), r"\??\C:\folder\file");
        let expected = std::env::current_dir().unwrap().join("file");
        assert_eq!(
            encoded(r"folder\..\file"),
            encoded(expected.to_str().unwrap())
        );
        assert_eq!(
            to_utf16(Path::new("bad\0path")).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}
