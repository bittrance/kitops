use std::{path::PathBuf, sync::Arc, thread::sleep, time::Duration};

use gix::ObjectId;

use crate::{errors::GitOpsError, task::ScheduledTask, workload::Workload};

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
    errfunc: Option<Arc<Box<dyn Fn() -> GitOpsError + Send + Sync>>>,
}

impl TestWorkload {
    pub fn fail_with(errfunc: impl Fn() -> GitOpsError + Send + Sync + 'static) -> Self {
        Self {
            errfunc: Some(Arc::new(Box::new(errfunc))),
            ..Default::default()
        }
    }
}

impl Workload for TestWorkload {
    fn id(&self) -> String {
        "test".to_string()
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(1)
    }

    fn perform(self, _workdir: PathBuf, _current_sha: ObjectId) -> Result<ObjectId, GitOpsError> {
        sleep(Duration::from_millis(10));
        if self.errfunc.is_some() {
            return Err(self.errfunc.unwrap()());
        }
        Ok(ObjectId::empty_blob(gix::hash::Kind::Sha1))
    }
}
