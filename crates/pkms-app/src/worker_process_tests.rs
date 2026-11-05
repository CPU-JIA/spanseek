use super::*;
use std::{
    io::{BufRead, Write},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{GetHandleInformation, HANDLE_FLAG_INHERIT, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
};

const FIXTURE: &str = "worker_process::tests::process_fixture";

fn fixture_command(role: &str, directory: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
        .args(["--exact", FIXTURE, "--ignored", "--nocapture"])
        .env("PKMS_WORKER_LIFETIME_FIXTURE", role)
        .env("PKMS_WORKER_LIFETIME_DIRECTORY", directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn wait_for_file(path: &Path) -> String {
    let started = Instant::now();
    loop {
        if let Ok(value) = std::fs::read_to_string(path)
            && !value.is_empty()
        {
            return value;
        }
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "fixture not ready: {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn process_handle(pid: u32) -> OwnedHandle {
    // Hold a handle before killing the parent to avoid PID-reuse observations.
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!handle.is_null(), "fixture process {pid} must be running");
    unsafe { OwnedHandle::from_raw_handle(handle) }
}

fn assert_running(handle: &OwnedHandle) {
    assert_eq!(
        unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) },
        WAIT_TIMEOUT
    );
}

fn assert_exited(handle: &OwnedHandle) {
    assert_eq!(
        unsafe { WaitForSingleObject(handle.as_raw_handle(), 5_000) },
        WAIT_OBJECT_0,
        "worker/descendant must exit when its owner's job handle closes"
    );
}

#[test]
fn hard_parent_exit_kills_worker_and_grandchild_in_nested_job() {
    let directory = tempfile::tempdir().unwrap();
    // This outer job stays open during all assertions. It provides cleanup if
    // an assertion fails and also tests operation under an existing outer job.
    let mut parent = WorkerChild::spawn(&mut fixture_command("parent", directory.path())).unwrap();
    let child_pid: u32 = wait_for_file(&directory.path().join("child.pid"))
        .parse()
        .unwrap();
    let leaf_pid: u32 = wait_for_file(&directory.path().join("leaf.pid"))
        .parse()
        .unwrap();
    let child = process_handle(child_pid);
    let leaf = process_handle(leaf_pid);
    assert_running(&child);
    assert_running(&leaf);
    // TerminateProcess only: bypass WorkerChild::kill and all parent destructors.
    parent.child.kill().unwrap();
    parent.child.wait().unwrap();
    assert!(
        parent.job.is_some(),
        "the outer cleanup job must remain open"
    );
    assert_exited(&child);
    assert_exited(&leaf);
}

pub(crate) fn assert_parent_kill(command: &mut Command, pid_file: &Path) {
    let mut parent = WorkerChild::spawn(command).unwrap();
    let pid = wait_for_file(pid_file).parse().unwrap();
    let child = process_handle(pid);
    assert_running(&child);
    parent.child.kill().unwrap();
    parent.child.wait().unwrap();
    assert!(parent.job.is_some());
    assert_exited(&child);
}

#[test]
fn assignment_failure_kills_and_reaps_the_unprotected_child() {
    use windows_sys::Win32::System::Threading::CreateEventW;
    let directory = tempfile::tempdir().unwrap();
    // A valid event handle causes a native type error without terminating the
    // target (unlike a job's active-process quota failure). Only this test can
    // construct a Job with a handle that did not come from CreateJobObjectW.
    let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
    assert!(!event.is_null());
    let job = windows::Job(unsafe { OwnedHandle::from_raw_handle(event) });
    // This fixture ignores stdin EOF, so closing pipes alone cannot pass.
    let child = fixture_command("leaf", directory.path()).spawn().unwrap();
    let cleanup_job = windows::Job::new().unwrap();
    cleanup_job.assign(&child).unwrap();
    let handle = process_handle(child.id());
    wait_for_file(&directory.path().join("leaf.pid"));
    assert!(job.assign(&child).is_err());
    assert_running(&handle);
    assert!(WorkerChild::attach(child, job).is_err());
    assert_exited(&handle);
    drop(cleanup_job);
}

#[test]
fn explicit_kill_releases_descendants_and_their_inherited_pipe() {
    let directory = tempfile::tempdir().unwrap();
    let mut command = fixture_command("child", directory.path());
    command.stdout(Stdio::piped());
    let mut child = WorkerChild::spawn(&mut command).unwrap();
    child.stdin.as_mut().unwrap().write_all(b"start\n").unwrap();
    let leaf_pid: u32 = wait_for_file(&directory.path().join("leaf.pid"))
        .parse()
        .unwrap();
    let leaf = process_handle(leaf_pid);
    assert_running(&leaf);
    let mut stdout = child.stdout.take().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = std::io::Read::read_to_end(&mut stdout, &mut bytes);
        let _ = send.send(result);
    });
    child.kill().unwrap();
    child.wait().unwrap();
    assert_exited(&leaf);
    receive
        .recv_timeout(Duration::from_secs(5))
        .expect("pipe must reach EOF")
        .unwrap();
    reader.join().unwrap();
}

#[test]
fn job_handle_is_not_inheritable_and_missing_executable_fails() {
    let job = windows::Job::new().unwrap();
    let mut flags = 0;
    assert_ne!(
        unsafe { GetHandleInformation(job.0.as_raw_handle(), &mut flags) },
        0
    );
    assert_eq!(flags & HANDLE_FLAG_INHERIT, 0);
    let directory = tempfile::tempdir().unwrap();
    assert!(WorkerChild::spawn(&mut Command::new(directory.path().join("missing.exe"))).is_err());
}

#[test]
#[ignore = "subprocess fixture; run only through lifecycle tests"]
fn process_fixture() {
    let role = std::env::var("PKMS_WORKER_LIFETIME_FIXTURE").unwrap();
    let directory = PathBuf::from(std::env::var_os("PKMS_WORKER_LIFETIME_DIRECTORY").unwrap());
    let _child;
    match role.as_str() {
        "parent" => {
            let mut child = WorkerChild::spawn(&mut fixture_command("child", &directory)).unwrap();
            child.stdin.as_mut().unwrap().write_all(b"start\n").unwrap();
            _child = child;
        }
        "child" => {
            let mut input = String::new();
            assert_ne!(std::io::stdin().lock().read_line(&mut input).unwrap(), 0);
            assert_eq!(input, "start\n");
            // A plain process spawn must inherit membership in the worker's job.
            let mut command = fixture_command("leaf", &directory);
            command.stdout(Stdio::inherit());
            let mut grandchild = command.spawn().unwrap();
            std::fs::write(directory.join("child.pid"), std::process::id().to_string()).unwrap();
            grandchild.wait().unwrap();
            panic!("fixture grandchild exited before its job was closed");
        }
        "leaf" => {
            std::fs::write(directory.join("leaf.pid"), std::process::id().to_string()).unwrap();
        }
        _ => panic!("unknown fixture role"),
    }
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}
