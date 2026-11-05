//! Native appearance follows committed preferences, never a frontend window capability.
use pkms_app::{Language, PreferencesState, Theme};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, Window, WindowEvent, window::Color};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

#[derive(Default)]
pub struct NativeAppearance(Mutex<AppearanceState>);

#[derive(Default)]
struct AppearanceState {
    preferences: Option<PreferencesState>,
}

impl AppearanceState {
    fn accept(&mut self, preferences: PreferencesState) -> bool {
        // Different request workers can finish out of order after committing.
        if self
            .preferences
            .as_ref()
            .is_some_and(|current| current.state_version > preferences.state_version)
        {
            return false;
        }
        self.preferences = Some(preferences);
        true
    }

    fn system_language(&self) -> Option<Language> {
        self.preferences.as_ref().and_then(|state| {
            (state.preferences.theme == Theme::System).then(|| state.preferences.language.clone())
        })
    }
}

fn native_theme(theme: &Theme) -> Option<tauri::Theme> {
    match theme {
        Theme::Light => Some(tauri::Theme::Light),
        Theme::Dark => Some(tauri::Theme::Dark),
        Theme::System => None,
    }
}

fn background(theme: tauri::Theme) -> Color {
    // Keep the unpainted WebView/window area aligned with the app's canvas.
    if theme == tauri::Theme::Dark {
        Color(33, 33, 33, 255)
    } else {
        Color(247, 247, 247, 255)
    }
}

/// Queue only after the preferences transaction has succeeded and its lock is released.
/// A native appearance failure must not turn an already committed save into an error.
pub fn apply_preferences(app: &AppHandle, preferences: PreferencesState) {
    let handle = app.clone();
    let language = preferences.preferences.language.clone();
    if app
        .run_on_main_thread(move || {
            let language = preferences.preferences.language.clone();
            if let Err(stage) = apply_on_main(&handle, preferences) {
                report_failure(&handle, &language, stage);
            }
        })
        .is_err()
    {
        report_failure(app, &language, "main_thread_unavailable");
    }
}

fn apply_on_main(app: &AppHandle, preferences: PreferencesState) -> Result<(), &'static str> {
    let appearance = app
        .try_state::<NativeAppearance>()
        .ok_or("appearance_unavailable")?;
    let theme = native_theme(&preferences.preferences.theme);
    {
        let mut state = appearance.0.lock().map_err(|_| "appearance_lock_failed")?;
        if !state.accept(preferences) {
            return Ok(());
        }
    }
    // Do not hold either the application service or appearance mutex while
    // calling native APIs: set_theme can synchronously deliver ThemeChanged.
    let window = app
        .get_webview_window("main")
        .ok_or("main_window_unavailable")?;
    window.set_theme(theme).map_err(|_| "set_theme_failed")?;
    let effective = match theme {
        Some(theme) => theme,
        None => window.theme().map_err(|_| "read_system_theme_failed")?,
    };
    window
        .set_background_color(Some(background(effective)))
        .map_err(|_| "set_background_failed")
}

/// Tauri keeps the system title bar and WebView2 theme in sync. Refresh the
/// explicit canvas color when the OS changes while "System" is selected.
pub fn window_event(window: &Window, event: &WindowEvent) {
    if window.label() != "main" || !matches!(event, WindowEvent::ThemeChanged(_)) {
        return;
    }
    let app = window.app_handle();
    let Some(appearance) = app.try_state::<NativeAppearance>() else {
        return; // Window creation can precede setup and preference loading.
    };
    let language = match appearance.0.lock() {
        Ok(state) => state.system_language(),
        Err(_) => {
            report_failure(app, &Language::default(), "appearance_lock_failed");
            return;
        }
    };
    let Some(language) = language else { return };
    let result = (|| {
        let window = app
            .get_webview_window("main")
            .ok_or("main_window_unavailable")?;
        // Query current state instead of applying a potentially older queued event.
        let effective = window.theme().map_err(|_| "read_system_theme_failed")?;
        window
            .set_background_color(Some(background(effective)))
            .map_err(|_| "set_background_failed")
    })();
    if let Err(stage) = result {
        report_failure(app, &language, stage);
    }
}

fn report_failure(app: &AppHandle, language: &Language, stage: &'static str) {
    // No paths, preference payloads or platform error strings in diagnostics.
    eprintln!("spanseek native appearance could not be applied: {stage}");
    let message = if *language == Language::English {
        "Your preferences are saved, but the native window appearance could not be updated. Restart spanseek to retry."
    } else {
        "偏好设置已保存，但原生窗口外观未能更新。请重新启动 spanseek 以重试。"
    };
    let mut dialog = app
        .dialog()
        .message(message)
        .title("spanseek")
        .kind(MessageDialogKind::Warning);
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.parent(&window);
    }
    // A native notice also works before the frontend's event listener is ready.
    dialog.show(|_| {});
}

#[cfg(test)]
mod tests {
    use super::*;
    use pkms_app::AppPreferences;

    fn preferences(version: u32, theme: Theme) -> PreferencesState {
        PreferencesState {
            state_version: version,
            preferences: AppPreferences {
                theme,
                ..AppPreferences::default()
            },
        }
    }

    #[test]
    fn delayed_startup_or_update_cannot_replace_newer_committed_preferences() {
        let mut state = AppearanceState::default();
        assert!(state.accept(preferences(4, Theme::Dark)));
        assert!(!state.accept(preferences(3, Theme::Light)));
        assert!(!state.accept(preferences(0, Theme::System)));
        assert_eq!(
            state.preferences.as_ref().unwrap().preferences.theme,
            Theme::Dark
        );
        // The same persisted state can be reapplied after a native API failure.
        assert!(state.accept(preferences(4, Theme::Dark)));
    }

    #[test]
    fn system_changes_only_repaint_when_system_is_selected() {
        let mut state = AppearanceState::default();
        assert_eq!(state.system_language(), None);
        assert!(state.accept(preferences(1, Theme::System)));
        assert_eq!(state.system_language(), Some(Language::SimplifiedChinese));
        assert_eq!(native_theme(&Theme::System), None);
        assert!(state.accept(preferences(2, Theme::Dark)));
        assert_eq!(state.system_language(), None);
        assert_eq!(native_theme(&Theme::Dark), Some(tauri::Theme::Dark));
        assert!(state.accept(preferences(3, Theme::Light)));
        assert_eq!(state.system_language(), None);
        assert_eq!(native_theme(&Theme::Light), Some(tauri::Theme::Light));
    }
}
