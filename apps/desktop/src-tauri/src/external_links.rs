//! OS navigation for validated external links and authorized completed outputs.
use pkms_app::{ApiError, ApiErrorCode, WorkspaceIdentity};
use tauri::{AppHandle, Manager, WebviewWindow};

fn validated_url(value: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if value.len() > 8192 || value.chars().any(char::is_control) {
        return Err(ApiError::new(ApiErrorCode::InvalidInput, ""));
    }
    let url =
        tauri::Url::parse(value).map_err(|_| ApiError::new(ApiErrorCode::InvalidInput, ""))?;
    match url.scheme() {
        "http" | "https" if url.host_str().is_some() => Ok(url.into()),
        "mailto" if !url.path().is_empty() => Ok(url.into()),
        _ => Err(ApiError::new(ApiErrorCode::InvalidInput, "")),
    }
}

#[cfg(target_os = "windows")]
fn open_target(value: &str) -> Result<(), ApiError> {
    use std::{ffi::c_void, iter::once, ptr};
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            hwnd: *mut c_void,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show: i32,
        ) -> *mut c_void;
    }
    let verb: Vec<u16> = "open".encode_utf16().chain(once(0)).collect();
    let target: Vec<u16> = value.encode_utf16().chain(once(0)).collect();
    // The validated URL is passed as a file argument. No shell command or
    // argument string is constructed; output folders use their dedicated API.
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            ptr::null(),
            ptr::null(),
            1,
        )
    } as isize;
    if result <= 32 {
        Err(ApiError::new(ApiErrorCode::InputUnavailable, ""))
    } else {
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn open_directory(value: &str) -> Result<(), ApiError> {
    let value = value.to_owned();
    // The shared blocking pool can already be initialized as MTA. Explorer's
    // folder operation needs its own STA rather than ignoring changed-mode.
    std::thread::Builder::new()
        .name("spanseek-output-folder".into())
        .spawn(move || open_directory_sta(&value))
        .map_err(|_| ApiError::new(ApiErrorCode::InputUnavailable, ""))?
        .join()
        .map_err(|_| ApiError::new(ApiErrorCode::InputUnavailable, ""))?
}

#[cfg(target_os = "windows")]
fn open_directory_sta(value: &str) -> Result<(), ApiError> {
    use std::{ffi::c_void, iter::once, ptr};
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHParseDisplayName(
            name: *const u16,
            binding: *mut c_void,
            item: *mut *mut c_void,
            requested_attributes: u32,
            attributes: *mut u32,
        ) -> i32;
        fn SHOpenFolderAndSelectItems(
            folder: *const c_void,
            count: u32,
            children: *const *const c_void,
            flags: u32,
        ) -> i32;
    }
    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoInitializeEx(reserved: *mut c_void, mode: u32) -> i32;
        fn CoUninitialize();
        fn CoTaskMemFree(memory: *mut c_void);
    }
    const FOLDER: u32 = 0x2000_0000;
    let initialized = unsafe { CoInitializeEx(ptr::null_mut(), 2) };
    #[cfg(debug_assertions)]
    eprintln!("output-folder COM status: {initialized:#010x}");
    if initialized < 0 {
        return Err(ApiError::new(ApiErrorCode::InputUnavailable, ""));
    }
    let name: Vec<u16> = value.encode_utf16().chain(once(0)).collect();
    let mut item = ptr::null_mut();
    let mut attributes = 0;
    let parsed = unsafe {
        SHParseDisplayName(
            name.as_ptr(),
            ptr::null_mut(),
            &mut item,
            FOLDER,
            &mut attributes,
        )
    };
    #[cfg(debug_assertions)]
    eprintln!(
        "output-folder parse status: {parsed:#010x}, folder: {}",
        attributes & FOLDER != 0
    );
    // A folder-specific shell operation cannot launch a same-name executable
    // even if a local filesystem actor replaces the path after validation.
    let opened = if parsed >= 0 && !item.is_null() && attributes & FOLDER != 0 {
        // cidl=0 selects this folder in its parent. An empty relative child
        // opens the authorized folder itself (SHOpenFolderAndSelectItems).
        let empty_child = 0u16;
        let children = [&empty_child as *const u16 as *const c_void];
        unsafe { SHOpenFolderAndSelectItems(item, 1, children.as_ptr(), 0) }
    } else {
        -1
    };
    #[cfg(debug_assertions)]
    eprintln!("output-folder open status: {opened:#010x}");
    if !item.is_null() {
        unsafe { CoTaskMemFree(item) };
    }
    if initialized >= 0 {
        unsafe { CoUninitialize() };
    }
    if opened < 0 {
        Err(ApiError::new(ApiErrorCode::InputUnavailable, ""))
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
fn open_directory(_value: &str) -> Result<(), ApiError> {
    Err(ApiError::new(ApiErrorCode::InputUnavailable, ""))
}

#[cfg(not(target_os = "windows"))]
fn open_target(value: &str) -> Result<(), ApiError> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let status = std::process::Command::new(program)
        .arg(value)
        .status()
        .map_err(|_| ApiError::new(ApiErrorCode::InputUnavailable, ""))?;
    if status.success() {
        Ok(())
    } else {
        Err(ApiError::new(ApiErrorCode::InputUnavailable, ""))
    }
}

#[tauri::command]
pub async fn open_external_url(window: WebviewWindow, href: String) -> Result<(), ApiError> {
    if window.label() != "main" {
        return Err(ApiError::new(ApiErrorCode::InvalidInput, ""));
    }
    let href = validated_url(&href)?;
    tauri::async_runtime::spawn_blocking(move || open_target(&href))
        .await
        .map_err(|_| ApiError::new(ApiErrorCode::Internal, ""))?
}

#[tauri::command]
pub async fn reveal_output_job(
    app: AppHandle,
    window: WebviewWindow,
    identity: WorkspaceIdentity,
    job_id: String,
) -> Result<(), ApiError> {
    if window.label() != "main" {
        return Err(ApiError::new(ApiErrorCode::InvalidInput, ""));
    }
    let service = app
        .try_state::<crate::runtime::DesktopRuntime>()
        .ok_or_else(|| ApiError::new(ApiErrorCode::Internal, ""))?
        .service
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        let directory = {
            let service = crate::runtime::lock(&service, "")?;
            service
                .published_output_directory(&identity, &job_id)
                .map_err(|error| {
                    let error = ApiError::from_app(error, "");
                    #[cfg(debug_assertions)]
                    eprintln!("output-folder authorization failed: {:?}", error.code);
                    error
                })?
        };
        #[cfg(debug_assertions)]
        eprintln!("output-folder authorization passed");
        let path = directory
            .to_str()
            .ok_or_else(|| ApiError::new(ApiErrorCode::InputUnavailable, ""))?
            .to_owned();
        // Explorer's directory handler uses Win32 paths, not the extended-length
        // filesystem prefix returned by canonicalize. Preserve UNC semantics.
        #[cfg(target_os = "windows")]
        let path = if let Some(unc) = path.strip_prefix("\\\\?\\UNC\\") {
            format!("\\\\{unc}")
        } else {
            path.strip_prefix("\\\\?\\").unwrap_or(&path).to_owned()
        };
        open_directory(&path)
    })
    .await
    .map_err(|_| ApiError::new(ApiErrorCode::Internal, ""))?
}

#[cfg(test)]
mod tests {
    use super::validated_url;

    #[test]
    fn external_links_allow_web_and_email_and_reject_local_or_executable_targets() {
        for value in [
            "https://example.com/path?q=%E4%B8%AD#section",
            "http://example.com",
            "mailto:person@example.com?subject=Hello",
        ] {
            assert!(validated_url(value).is_ok(), "{value}");
        }
        for value in [
            "",
            "#anchor",
            "file:///C:/Windows/system32/cmd.exe",
            "javascript:alert(1)",
            "data:text/html,test",
            "cmd:/c",
            "mailto:",
            "https://example.com\nanything",
            "C:\\Windows\\notepad.exe",
        ] {
            assert!(validated_url(value).is_err(), "{value}");
        }
    }
}
