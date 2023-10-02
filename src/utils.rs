use std::ops::Deref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

#[cfg(test)]
pub const POLL_INTERVAL: Duration = Duration::from_millis(10);
#[cfg(not(test))]
pub const POLL_INTERVAL: Duration = Duration::from_millis(500);

pub struct Watchdog {
    deadline: Instant,
    cancellation: AtomicBool,
}

impl Watchdog {
    pub fn new(deadline: Instant) -> Self {
        Self {
            deadline,
            cancellation: AtomicBool::new(false),
        }
    }

    pub fn runner(&self) -> impl FnOnce() + '_ {
        || {
            while Instant::now() < self.deadline && !self.cancellation.load(Ordering::Relaxed) {
                sleep(POLL_INTERVAL);
            }
            self.cancellation.store(true, Ordering::Relaxed);
        }
    }

    pub fn cancel(&self) {
        self.cancellation.store(true, Ordering::Relaxed);
    }
}

impl Deref for Watchdog {
    type Target = AtomicBool;

    fn deref(&self) -> &Self::Target {
        &self.cancellation
    }
}

#[cfg(test)]
mod tests {
    use std::thread::scope;
    use std::time::Instant;

    #[test]
    fn reach_deadline() {
        let deadline = Instant::now() + super::POLL_INTERVAL * 3;
        let watchdog = super::Watchdog::new(deadline);
        watchdog.runner()();
        assert!(Instant::now() > deadline);
    }

    #[test]
    fn cancellation() {
        let deadline = Instant::now() + super::POLL_INTERVAL * 3;
        let watchdog = super::Watchdog::new(deadline);
        scope(|s| {
            s.spawn(watchdog.runner());
            watchdog.cancel();
        });
        assert!(Instant::now() < deadline);
    }
}
