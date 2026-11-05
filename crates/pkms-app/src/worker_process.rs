//! Lifetime ownership for the two stdin-driven worker protocols.
use std::{
    io,
    ops::{Deref, DerefMut},
    process::{Child, Command},
};

pub(crate) struct WorkerChild {
    child: Child,
    #[cfg(windows)]
    job: Option<windows::Job>,
}

impl WorkerChild {
    pub(crate) fn spawn(command: &mut Command) -> io::Result<Self> {
        #[cfg(windows)]
        let job = windows::Job::new()?;
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
        }
        #[cfg(windows)]
        {
            Self::attach(command.spawn()?, job)
        }
        #[cfg(not(windows))]
        {
            Ok(Self {
                child: command.spawn()?,
            })
        }
    }

    #[cfg(windows)]
    fn attach(child: Child, job: windows::Job) -> io::Result<Self> {
        let child = Self {
            child,
            job: Some(job),
        };
        // No business input is written until assignment succeeds. Both workers
        // exit on stdin EOF if the parent dies between spawn and assignment.
        // An assignment error drops this guard and kills/reaps the new process.
        child.job.as_ref().expect("new job").assign(&child.child)?;
        Ok(child)
    }

    pub(crate) fn kill(&mut self) -> io::Result<()> {
        // Close before joining any pipe threads: descendants may hold inherited
        // pipe ends. The job handle itself is never inherited by a child.
        #[cfg(windows)]
        self.job.take();
        self.child.kill()
    }
}

impl Deref for WorkerChild {
    type Target = Child;
    fn deref(&self) -> &Self::Target {
        &self.child
    }
}

impl DerefMut for WorkerChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.child
    }
}

impl Drop for WorkerChild {
    fn drop(&mut self) {
        let _ = self.kill();
        let _ = self.child.wait();
    }
}

#[cfg(windows)]
mod windows {
    use std::{
        io,
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::Child,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    pub(super) struct Job(pub(super) OwnedHandle);

    impl Job {
        pub(super) fn new() -> io::Result<Self> {
            // NULL security attributes make the unnamed handle non-inheritable.
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: CreateJobObjectW returned a new owned, valid handle.
            let job = Self(unsafe { OwnedHandle::from_raw_handle(handle) });
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: The handle and correctly sized information buffer live
            // through this synchronous call. No BREAKAWAY flags are enabled.
            if unsafe {
                SetInformationJobObject(
                    job.0.as_raw_handle(),
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(job)
        }

        pub(super) fn assign(&self, child: &Child) -> io::Result<()> {
            // SAFETY: Both handles remain valid for the call. On Windows 8+
            // this nests within a compatible outer job; restrictions propagate.
            // Incompatible outer jobs fail explicitly instead of breaking away.
            if unsafe { AssignProcessToJobObject(self.0.as_raw_handle(), child.as_raw_handle()) }
                == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
    }
}

#[cfg(all(test, windows))]
#[path = "worker_process_tests.rs"]
pub(crate) mod tests;
