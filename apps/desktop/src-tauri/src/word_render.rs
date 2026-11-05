//! Word-exact DOCX rendering through a local Microsoft Word instance.
//!
//! The bundled semantic renderer is deliberately independent of Microsoft
//! Office, but Word splits tables, resolves section breaks and lays out fields
//! with its own pagination engine. When Word is installed we use that local
//! engine, read-only and with macros disabled, to produce a PDF that the
//! existing PDF reader can display. A machine without Word keeps the semantic
//! fallback; no document content leaves the machine.

use std::path::Path;

#[derive(Debug)]
pub enum WordRenderError {
    /// Word is not registered on this machine; the caller should use the
    /// semantic fallback instead of treating this as a document failure.
    NotInstalled,
    Failed(String),
}

impl std::fmt::Display for WordRenderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInstalled => formatter.write_str("Microsoft Word is not installed"),
            Self::Failed(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for WordRenderError {}

pub fn render_docx_to_pdf(input: &Path, output: &Path) -> Result<(), WordRenderError> {
    windows_impl::render(input, output)
}

#[cfg(not(windows))]
mod windows_impl {
    use super::WordRenderError;
    use std::path::Path;

    pub(super) fn render(_: &Path, _: &Path) -> Result<(), WordRenderError> {
        Err(WordRenderError::NotInstalled)
    }
}

#[cfg(windows)]
mod windows_impl {
    use super::WordRenderError;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows::Win32::System::Com::{
        CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize, DISPATCH_FLAGS, DISPATCH_METHOD, DISPATCH_PROPERTYGET,
        DISPATCH_PROPERTYPUT, DISPPARAMS, IDispatch,
    };
    use windows::Win32::System::Ole::DISPID_PROPERTYPUT;
    use windows::Win32::System::Variant::VARIANT;
    use windows::core::{BSTR, GUID, PCWSTR};

    const LCID_EN_US: u32 = 0x0409;
    const WD_FORMAT_PDF: i32 = 17;
    /// `msoAutomationSecurityForceDisable`: macros in an imported document are
    /// disabled before Word opens it.
    const AUTOMATION_SECURITY_FORCE_DISABLE: i32 = 3;

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn variant_path(path: &Path) -> VARIANT {
        let value: Vec<u16> = path.as_os_str().encode_wide().collect();
        VARIANT::from(BSTR::from_wide(&value))
    }

    fn null_iid() -> GUID {
        GUID::from_u128(0)
    }

    fn dispid(object: &IDispatch, name: &str) -> windows::core::Result<i32> {
        let name = wide(name);
        let names = [PCWSTR::from_raw(name.as_ptr())];
        let mut result = 0;
        unsafe {
            object.GetIDsOfNames(&null_iid(), names.as_ptr(), 1, LCID_EN_US, &mut result)?;
        }
        Ok(result)
    }

    fn invoke(
        object: &IDispatch,
        name: &str,
        flags: DISPATCH_FLAGS,
        mut arguments: Vec<VARIANT>,
    ) -> windows::core::Result<VARIANT> {
        let id = dispid(object, name)?;
        // COM passes positional arguments in reverse order.
        arguments.reverse();
        let parameters = DISPPARAMS {
            rgvarg: arguments.as_mut_ptr(),
            rgdispidNamedArgs: std::ptr::null_mut(),
            cArgs: arguments.len() as u32,
            cNamedArgs: 0,
        };
        let mut result = VARIANT::default();
        unsafe {
            object.Invoke(
                id,
                &null_iid(),
                LCID_EN_US,
                flags,
                &parameters,
                Some(&mut result),
                None,
                None,
            )?;
        }
        Ok(result)
    }

    fn invoke_dispatch(
        object: &IDispatch,
        name: &str,
        arguments: Vec<VARIANT>,
    ) -> windows::core::Result<IDispatch> {
        let value = invoke(object, name, DISPATCH_METHOD, arguments)?;
        IDispatch::try_from(&value)
    }

    fn invoke_void(
        object: &IDispatch,
        name: &str,
        arguments: Vec<VARIANT>,
    ) -> windows::core::Result<()> {
        let _ = invoke(object, name, DISPATCH_METHOD, arguments)?;
        Ok(())
    }

    fn get_dispatch(object: &IDispatch, name: &str) -> windows::core::Result<IDispatch> {
        let value = invoke(object, name, DISPATCH_PROPERTYGET, Vec::new())?;
        IDispatch::try_from(&value)
    }

    fn put_property(object: &IDispatch, name: &str, value: VARIANT) -> windows::core::Result<()> {
        let id = dispid(object, name)?;
        let mut arguments = [value];
        let mut named = [DISPID_PROPERTYPUT];
        let parameters = DISPPARAMS {
            rgvarg: arguments.as_mut_ptr(),
            rgdispidNamedArgs: named.as_mut_ptr(),
            cArgs: 1,
            cNamedArgs: 1,
        };
        unsafe {
            object.Invoke(
                id,
                &null_iid(),
                LCID_EN_US,
                DISPATCH_PROPERTYPUT,
                &parameters,
                None,
                None,
                None,
            )?;
        }
        Ok(())
    }

    struct WordSession {
        app: IDispatch,
        document: Option<IDispatch>,
        quit: bool,
    }

    impl Drop for WordSession {
        fn drop(&mut self) {
            if let Some(document) = self.document.take() {
                let _ = invoke_void(&document, "Close", vec![VARIANT::from(false)]);
            }
            if !self.quit {
                let _ = invoke_void(&self.app, "Quit", vec![VARIANT::from(false)]);
            }
        }
    }

    fn failed(error: windows::core::Error) -> WordRenderError {
        WordRenderError::Failed(format!("Word automation failed ({error})"))
    }

    fn render_inner(input: &Path, output: &Path) -> Result<(), WordRenderError> {
        let program = wide("Word.Application");
        let class = unsafe {
            windows::Win32::System::Com::CLSIDFromProgID(PCWSTR::from_raw(program.as_ptr()))
        }
        .map_err(|_| WordRenderError::NotInstalled)?;
        let app: IDispatch = unsafe {
            CoCreateInstance(
                &class,
                None::<&windows::core::IUnknown>,
                CLSCTX_LOCAL_SERVER,
            )
        }
        .map_err(failed)?;
        let mut session = WordSession {
            app: app.clone(),
            document: None,
            quit: false,
        };

        // These properties are safety requirements, not cosmetic settings:
        // the document is opened read-only and macros are disabled.
        put_property(
            &app,
            "AutomationSecurity",
            VARIANT::from(AUTOMATION_SECURITY_FORCE_DISABLE),
        )
        .map_err(failed)?;
        put_property(&app, "DisplayAlerts", VARIANT::from(0i32)).map_err(failed)?;
        put_property(&app, "Visible", VARIANT::from(false)).map_err(failed)?;
        let _ = put_property(&app, "ScreenUpdating", VARIANT::from(false));
        if let Ok(options) = get_dispatch(&app, "Options") {
            let _ = put_property(&options, "UpdateLinksAtOpen", VARIANT::from(false));
            let _ = put_property(&options, "ConfirmConversions", VARIANT::from(false));
        }

        let documents = get_dispatch(&app, "Documents").map_err(failed)?;
        let document = invoke_dispatch(
            &documents,
            "Open",
            vec![
                variant_path(input),
                VARIANT::from(false), // ConfirmConversions
                VARIANT::from(true),  // ReadOnly
                VARIANT::from(false), // AddToRecentFiles
            ],
        )
        .map_err(failed)?;
        session.document = Some(document.clone());

        invoke_void(
            &document,
            "SaveAs2",
            vec![variant_path(output), VARIANT::from(WD_FORMAT_PDF)],
        )
        .map_err(failed)?;
        invoke_void(&document, "Close", vec![VARIANT::from(false)]).map_err(failed)?;
        session.document = None;
        invoke_void(&app, "Quit", vec![VARIANT::from(false)]).map_err(failed)?;
        session.quit = true;
        Ok(())
    }

    pub(super) fn render(input: &Path, output: &Path) -> Result<(), WordRenderError> {
        let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if initialized.is_err() {
            return Err(WordRenderError::Failed(format!(
                "COM initialization failed ({initialized:?})"
            )));
        }
        let result = render_inner(input, output);
        unsafe { CoUninitialize() };
        result
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::render_docx_to_pdf;
    use std::path::Path;

    #[test]
    #[ignore = "requires a local Microsoft Word and PKMS_TEST_DOCX"]
    fn local_word_exports_a_pdf() {
        let input = std::env::var_os("PKMS_TEST_DOCX").expect("set PKMS_TEST_DOCX");
        let output =
            std::env::temp_dir().join(format!("spanseek-word-render-{}.pdf", uuid::Uuid::new_v4()));
        render_docx_to_pdf(Path::new(&input), &output).expect("Word PDF export");
        let bytes = std::fs::read(&output).expect("read PDF");
        assert!(bytes.starts_with(b"%PDF-"));
        let _ = std::fs::remove_file(output);
    }
}
