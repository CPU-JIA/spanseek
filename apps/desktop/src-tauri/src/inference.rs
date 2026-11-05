use crate::runtime::{AppDataDirectory, DesktopRuntime, bundled_sidecar, emit, lock};
use pkms_app::{ApiError, ApiErrorCode, AppService, DesktopEvent, InferenceConfig};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};

pub fn config(app: &AppHandle, runtime: &DesktopRuntime) -> Result<InferenceConfig, ApiError> {
    let directory = app.state::<AppDataDirectory>().0.clone();
    let executable = if cfg!(debug_assertions) {
        std::env::var_os("PKMS_INFERENCE_EXE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                bundled_sidecar(
                    &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug"),
                    "spanseek-inference.exe",
                    "pkms-inference.exe",
                )
            })
    } else {
        let directory = std::env::current_exe()
            .map_err(|_| ApiError::new(ApiErrorCode::InputUnavailable, ""))?
            .parent()
            .ok_or_else(|| ApiError::new(ApiErrorCode::InputUnavailable, ""))?
            .to_path_buf();
        bundled_sidecar(&directory, "spanseek-inference.exe", "pkms-inference.exe")
    };
    let model_directory = if cfg!(debug_assertions) {
        std::env::var_os("PKMS_EMBEDDING_DIR").map(std::path::PathBuf::from)
    } else {
        None
    }
    .unwrap_or(directory.join("models/multilingual-e5-small"));
    Ok(InferenceConfig {
        executable,
        model_directory,
        vector_root: directory.join("vectors"),
        shutdown: runtime.source_shutdown.clone(),
    })
}

/// One native scheduler wakes due durable retries even when no document is edited.
pub fn start_scheduler(app: &AppHandle) {
    let runtime = app.state::<DesktopRuntime>().inner().clone();
    let handle = app.clone();
    let mut threads = runtime
        .inference_threads
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let shutdown = runtime.source_shutdown.clone();
    threads.push(std::thread::spawn(move || {
        let mut last = std::time::Instant::now();
        while !shutdown.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(500));
            if last.elapsed() < std::time::Duration::from_secs(30) {
                continue;
            }
            last = std::time::Instant::now();
            let Some(runtime) = handle.try_state::<DesktopRuntime>() else {
                return;
            };
            if let Ok(service) = runtime.service.lock() {
                resume(&handle, &service);
            }
        }
    }));
}

pub fn resume(app: &AppHandle, service: &AppService) {
    let Some(runtime) = app.try_state::<DesktopRuntime>() else {
        return;
    };
    if runtime.source_shutdown.load(Ordering::Acquire) {
        return;
    }
    let setup = (|| {
        let config = config(app, &runtime)?;
        let worker = service
            .inference_jobs_worker()
            .map_err(|error| ApiError::from_app(error, ""))?;
        Ok::<_, ApiError>((config, worker))
    })();
    let Ok((config, Some(worker))) = setup else {
        if setup.is_err() {
            eprintln!("spanseek inference worker could not start; durable task state is retained.");
        }
        return;
    };
    let Ok(mut threads) = lock(&runtime.inference_threads, "") else {
        return;
    };
    if runtime.source_shutdown.load(Ordering::Acquire) {
        return;
    }
    let mut pending = Vec::new();
    for thread in threads.drain(..) {
        if thread.is_finished() {
            let _ = thread.join();
        } else {
            pending.push(thread);
        }
    }
    let handle = app.clone();
    pending.push(std::thread::spawn(move || {
        if worker
            .run_with_chat_progress(
                config,
                &mut |identity, answer, rebuild| {
                    emit(
                        &handle,
                        DesktopEvent::InferenceChanged {
                            identity: identity.clone(),
                            answer_id: answer.map(|answer| answer.id.clone()),
                            rebuild_id: rebuild.map(|job| job.id.clone()),
                        },
                    );
                },
                &mut |identity, progress| {
                    emit(
                        &handle,
                        DesktopEvent::AgentProgress {
                            identity: identity.clone(),
                            progress: progress.clone(),
                        },
                    );
                },
                &mut |identity, run| {
                    emit(
                        &handle,
                        DesktopEvent::ChatChanged {
                            identity: identity.clone(),
                            conversation_id: run.conversation_id.clone(),
                            run_id: run.id.clone(),
                            last_event_seq: run.last_event_seq,
                        },
                    );
                },
                &mut |identity, progress| {
                    emit(
                        &handle,
                        DesktopEvent::ChatProgress {
                            identity: identity.clone(),
                            progress: progress.clone(),
                        },
                    );
                },
            )
            .is_err()
        {
            eprintln!("spanseek inference worker stopped; durable task state is retained.");
        }
    }));
    *threads = pending;
}
