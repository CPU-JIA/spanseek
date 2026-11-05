//! The app owns a single expensive-compute slot; waiting never holds a database mutex.
use crate::{AppError, Result};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

#[derive(Default)]
pub(crate) struct HeavyResourceSlot {
    occupied: Mutex<bool>,
    changed: Condvar,
}
pub(crate) struct HeavyResourcePermit<'a>(&'a HeavyResourceSlot);
impl Drop for HeavyResourcePermit<'_> {
    fn drop(&mut self) {
        *self
            .0
            .occupied
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = false;
        self.0.changed.notify_one();
    }
}
impl HeavyResourceSlot {
    pub(crate) fn acquire(
        &self,
        check: &mut dyn FnMut() -> Result<()>,
    ) -> Result<HeavyResourcePermit<'_>> {
        loop {
            check()?;
            let mut occupied = self.occupied.lock().map_err(|_| AppError::AppBusy)?;
            if !*occupied {
                *occupied = true;
                return Ok(HeavyResourcePermit(self));
            }
            let _ = self
                .changed
                .wait_timeout(occupied, Duration::from_millis(100))
                .map_err(|_| AppError::AppBusy)?;
        }
    }
}
pub(crate) fn heavy_resource_slot() -> &'static HeavyResourceSlot {
    static SLOT: OnceLock<HeavyResourceSlot> = OnceLock::new();
    SLOT.get_or_init(HeavyResourceSlot::default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    #[test]
    fn waiting_work_cancels_without_stealing_the_live_slot() {
        let slot = Arc::new(HeavyResourceSlot::default());
        let permit = slot.acquire(&mut || Ok(())).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let waiting_slot = slot.clone();
        let waiting_cancel = cancel.clone();
        let worker = std::thread::spawn(move || {
            let result = waiting_slot.acquire(&mut || {
                if waiting_cancel.load(Ordering::Acquire) {
                    Err(AppError::InvalidInput("canceled"))
                } else {
                    Ok(())
                }
            });
            assert!(matches!(result, Err(AppError::InvalidInput("canceled"))));
        });
        cancel.store(true, Ordering::Release);
        worker.join().unwrap();
        assert!(*slot.occupied.lock().unwrap());
        drop(permit);
        assert!(slot.acquire(&mut || Ok(())).is_ok());
    }
}
