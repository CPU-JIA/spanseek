mod appearance;
mod commands;
mod docx_render;
mod external_documents;
mod external_links;
mod images;
mod inference;
mod runtime;
mod source_resources;
mod thumbnail;
mod window_geometry;
mod word_render;
use runtime::DesktopRuntime;

/// WebView2 raises `PermissionRequested` for getUserMedia. Grant only the
/// microphone for this application's own webview; camera and notifications keep
/// the runtime's default behavior.
#[cfg(windows)]
fn grant_microphone_permission(window: &tauri::WebviewWindow) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PERMISSION_KIND_MICROPHONE, COREWEBVIEW2_PERMISSION_KIND_UNKNOWN_PERMISSION,
        COREWEBVIEW2_PERMISSION_STATE_ALLOW, ICoreWebView2_13, ICoreWebView2Profile4,
    };
    use webview2_com::PermissionRequestedEventHandler;
    use windows::core::{HSTRING, Interface, PCWSTR};
    let _ = window.with_webview(|webview| {
        let controller = webview.controller();
        let core = match unsafe { controller.CoreWebView2() } {
            Ok(core) => core,
            Err(_) => return,
        };
        // A previous denial is persisted in the WebView2 profile, so the event
        // below would never fire again. Reset the microphone decision for this
        // app's own origins before the page asks for it.
        if let Ok(core) = core.cast::<ICoreWebView2_13>()
            && let Ok(profile) = unsafe { core.Profile() }
            && let Ok(profile) = profile.cast::<ICoreWebView2Profile4>()
        {
            for origin in [
                "http://127.0.0.1:5173",
                "http://tauri.localhost",
                "https://tauri.localhost",
            ] {
                let origin = HSTRING::from(origin);
                let _ = unsafe {
                    profile.SetPermissionState(
                        COREWEBVIEW2_PERMISSION_KIND_MICROPHONE,
                        PCWSTR(origin.as_ptr()),
                        COREWEBVIEW2_PERMISSION_STATE_ALLOW,
                        None,
                    )
                };
            }
        }
        let handler = PermissionRequestedEventHandler::create(Box::new(|_sender, args| {
            if let Some(args) = args {
                let mut kind = COREWEBVIEW2_PERMISSION_KIND_UNKNOWN_PERMISSION;
                if unsafe { args.PermissionKind(&mut kind) }.is_ok()
                    && kind == COREWEBVIEW2_PERMISSION_KIND_MICROPHONE
                {
                    let _ = unsafe { args.SetState(COREWEBVIEW2_PERMISSION_STATE_ALLOW) };
                }
            }
            Ok(())
        }));
        let mut token = 0;
        let _ = unsafe { core.add_PermissionRequested(&handler, &mut token) };
    });
}

#[cfg(not(windows))]
fn grant_microphone_permission(_window: &tauri::WebviewWindow) {}

/// Ctrl+Alt+V opens voice capture even when another window has focus. A
/// dedicated thread owns the Win32 message loop; the hotkey lives until exit.
#[cfg(windows)]
fn register_voice_hotkey(app: tauri::AppHandle) {
    use tauri::Emitter;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, RegisterHotKey,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};
    std::thread::spawn(move || unsafe {
        let modifiers: HOT_KEY_MODIFIERS = MOD_CONTROL | MOD_ALT;
        if RegisterHotKey(None, 1, modifiers, u32::from(b'V')).is_err() {
            return;
        }
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            if message.message == WM_HOTKEY {
                let _ = app.emit("voice-capture", ());
            }
        }
    });
}

#[cfg(not(windows))]
fn register_voice_hotkey(_app: tauri::AppHandle) {}

/// Tray entry mirrors the toolbar: open voice capture, restore the window, or
/// quit. The icon is the same brand icon bundled with the app.
fn install_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::Emitter;
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    let voice = MenuItem::with_id(app, "voice", "语音速记", true, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&voice, &show, &quit])?;
    let mut builder = TrayIconBuilder::with_id("spanseek")
        .tooltip("跨索 spanseek")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "voice" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
                let _ = app.emit("voice-capture", ());
            }
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
                && let Some(window) = tray.app_handle().get_webview_window("main")
            {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        });
    if let Some(icon) = app
        .default_window_icon()
        .cloned()
        .or_else(|| tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png")).ok())
    {
        builder = builder.icon(icon);
    }
    if let Err(error) = builder.build(app) {
        eprintln!("tray init failed: {error}");
        return Err(error);
    }
    Ok(())
}

const TEST_APP_DATA_GATE: &str = "PKMS_ENABLE_TEST_APP_DATA";
fn isolated_app_data_directory() -> Option<std::path::PathBuf> {
    if std::env::var(TEST_APP_DATA_GATE).ok().as_deref() != Some("1") {
        return None;
    }
    std::env::var_os("PKMS_APP_DATA")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
}
use tauri::Manager;

pub fn run() {
    let builder = tauri::Builder::default();
    // Isolated acceptance runs need two app identities on the same machine.
    // This escape hatch is compiled only into debug builds and is never read by
    // a release package; normal launches keep the single-instance guard.
    let builder =
        if cfg!(debug_assertions) && std::env::var_os("PKMS_ALLOW_MULTI_INSTANCE").is_some() {
            builder
        } else {
            builder.plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
                external_documents::receive_arguments(app, args, cwd.into());
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }))
        };
    let result = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            commands::desktop_request,
            commands::voice_capture_audio,
            commands::voice_capture_export,
            commands::choose_resource,
            commands::choose_source_files,
            commands::choose_source_directory,
            commands::retry_startup,
            external_links::open_external_url,
            external_links::reveal_output_job,
            external_documents::choose_open_documents,
            external_documents::take_open_documents,
            external_documents::release_open_document,
            external_documents::grant_open_documents,
            window_geometry::read_window_restore_geometry,
            source_resources::issue_source_resource,
            source_resources::release_source_resource,
            docx_render::issue_docx_word_preview,
            commands::ocr_status
        ])
        .on_window_event(|window, event| {
            appearance::window_event(window, event);
            if window.label() == "main"
                && let tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) =
                    event
            {
                external_documents::receive_paths(window.app_handle(), paths.clone());
            }
        })
        .setup(|app| {
            if let Ok(version) = tauri::webview_version() {
                pkms_app::set_diagnostic_webview_version(&version);
            }
            let directory =
                isolated_app_data_directory().unwrap_or(app.path().local_data_dir()?.join("PKMS"));
            app.manage(runtime::StartupFailure::default());
            app.manage(runtime::AppDataDirectory(directory.clone()));
            app.manage(appearance::NativeAppearance::default());
            if let Some(window) = app.get_webview_window("main") {
                grant_microphone_permission(&window);
            }
            register_voice_hotkey(app.handle().clone());
            let _ = install_tray(app.handle());
            external_documents::receive_arguments(
                app.handle(),
                std::env::args().collect(),
                std::env::current_dir().unwrap_or_default(),
            );
            let handle = app.handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                // A failure is remembered on the handle; `retry_startup` clears
                // it and calls the same entry point again.
                let _ = runtime::initialize(&handle, directory);
            });
            Ok(())
        })
        .register_asynchronous_uri_scheme_protocol("pkms-image", |context, request, responder| {
            let runtime = match runtime::current(context.app_handle(), "") {
                Ok(runtime) => runtime,
                Err(_) => {
                    responder.respond(tauri::http::Response::new(b"Storage unavailable".to_vec()));
                    return;
                }
            };
            tauri::async_runtime::spawn_blocking(move || {
                responder.respond(images::read_image(&runtime, request))
            });
        })
        .register_asynchronous_uri_scheme_protocol("pkms-source", |context, request, responder| {
            if request.uri().path().starts_with("/local/") {
                tauri::async_runtime::spawn_blocking(move || {
                    responder.respond(external_documents::read_resource(request))
                });
                return;
            }
            let runtime = match runtime::current(context.app_handle(), "") {
                Ok(runtime) => runtime,
                Err(_) => {
                    let mut response = tauri::http::Response::new(Vec::<u8>::new());
                    *response.status_mut() = tauri::http::StatusCode::SERVICE_UNAVAILABLE;
                    responder.respond(response);
                    return;
                }
            };
            tauri::async_runtime::spawn_blocking(move || {
                responder.respond(source_resources::read_source_resource(&runtime, request))
            });
        })
        .build(tauri::generate_context!());
    let Ok(app) = result else {
        // No path, SQL, or user content in startup diagnostics.
        eprintln!("spanseek could not start. Check the application data lock and desktop runtime.");
        std::process::exit(1);
    };
    app.run(|handle, event| {
        if matches!(
            event,
            tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
        ) && let Some(runtime) = handle.try_state::<DesktopRuntime>()
        {
            runtime.shutdown_sources();
        }
    });
}
