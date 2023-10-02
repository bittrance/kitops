use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
    thread::sleep,
    time::Duration,
};

use gix::ObjectId;

use crate::{
    errors::GitOpsError,
    task::{scheduled::ScheduledTask, Workload},
};

impl<W: Workload + Clone + Send + 'static> ScheduledTask<W> {
    pub fn await_finished(&self) {
        while !self.is_finished() {
            sleep(Duration::from_millis(2));
        }
    }

    pub fn await_eligible(&self) {
        while !self.is_eligible() {
            sleep(Duration::from_millis(2));
        }
    }
}

#[derive(Clone, Default)]
pub struct TestWorkload {
    pub status: Arc<AtomicBool>,
}

impl Workload for TestWorkload {
    fn id(&self) -> String {
        "test".to_string()
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(1)
    }

    fn perform(self, _workdir: PathBuf, _current_sha: ObjectId) -> Result<ObjectId, GitOpsError> {
        self.status
            .store(true, std::sync::atomic::Ordering::Relaxed);
        sleep(Duration::from_millis(10));
        Ok(ObjectId::empty_blob(gix::hash::Kind::Sha1))
    }
}
