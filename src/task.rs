use std::{
    ops::Add,
    thread::{spawn, JoinHandle},
    time::SystemTime,
};

use gix::ObjectId;

use crate::{errors::GitOpsError, state::State, workload::Workload};

pub struct ScheduledTask<W: Workload + Clone + Send> {
    work: W,
    pub state: State,
    worker: Option<JoinHandle<Result<ObjectId, GitOpsError>>>,
}

impl<W: Workload + Clone + Send + 'static> ScheduledTask<W> {
    pub fn new(work: W) -> Self {
        Self {
            work,
            state: State::default(),
            worker: None,
        }
    }

    pub fn id(&self) -> String {
        self.work.id()
    }

    pub fn is_eligible(&self) -> bool {
        self.worker.is_none() && SystemTime::now() >= self.state.next_run
    }

    pub fn is_running(&self) -> bool {
        self.worker.as_ref().is_some_and(|h| !h.is_finished())
    }

    pub fn is_finished(&self) -> bool {
        self.worker.as_ref().is_some_and(|h| h.is_finished())
    }

    pub fn schedule_next(&mut self) {
        self.state.next_run = SystemTime::now().add(self.work.interval());
    }

    pub fn start(&mut self) -> Result<(), GitOpsError> {
        let current_sha = self.state.current_sha;
        let workdir = tempfile::tempdir()
            .map_err(GitOpsError::WorkDir)?
            .into_path();
        let work = self.work.clone();
        self.worker = Some(spawn(move || work.perform(workdir, current_sha)));
        Ok(())
    }

    pub fn finalize(&mut self) -> Result<(), GitOpsError> {
        let new_sha = self
            .worker
            .take()
            .expect("result only called once")
            .join()
            .expect("thread not to panic")?;
        self.state.current_sha = new_sha;
        Ok(())
    }

    pub fn state(&self) -> State {
        self.state.clone()
    }

    pub fn set_state(&mut self, state: State) {
        self.state = state;
        // If configuration has changed, this will move up the next run
        self.state.next_run = std::cmp::min(
            self.state.next_run,
            SystemTime::now().add(self.work.interval()),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::{
        thread::sleep,
        time::{Duration, SystemTime},
    };

    use gix::ObjectId;

    use crate::{state::State, task::ScheduledTask, testutils::TestWorkload};

    #[test]
    fn scheduled_task_flow() {
        let mut task = ScheduledTask::new(TestWorkload::default());
        assert!(task.is_eligible());
        assert!(!task.is_running());
        assert!(!task.is_finished());
        task.start().unwrap();
        assert!(!task.is_eligible());
        assert!(task.is_running());
        assert!(!task.is_finished());
        task.await_finished();
        assert!(!task.is_eligible());
        assert!(!task.is_running());
        task.finalize().unwrap();
        assert!(!task.is_finished());
        assert!(task.state().current_sha.is_empty_blob());
        task.await_eligible();
    }

    #[test]
    #[should_panic]
    fn scheduled_task_on_panic() {
        let mut task = ScheduledTask::new(TestWorkload::fail_with(|| panic!("BOOM!")));
        task.start().unwrap();
        task.await_finished();
        assert!(!task.is_running());
        task.finalize().unwrap();
    }

    #[test]
    fn scheduled_task_on_existing_state() {
        let mut task = ScheduledTask::new(TestWorkload::default());
        task.set_state(State {
            current_sha: ObjectId::null(gix::hash::Kind::Sha1),
            next_run: SystemTime::now() + Duration::from_millis(10),
        });
        assert!(!task.is_eligible());
        sleep(Duration::from_millis(10));
        assert!(task.is_eligible());
    }

    #[test]
    fn set_state_picks_earliest_next_run() {
        let stored_next_run = SystemTime::now();
        let mut task = ScheduledTask::new(TestWorkload::default());
        task.set_state(State {
            current_sha: ObjectId::null(gix::hash::Kind::Sha1),
            next_run: stored_next_run,
        });
        assert!(task.state().next_run == stored_next_run);
        let stored_next_run = SystemTime::now() + Duration::from_secs(10);
        task.set_state(State {
            current_sha: ObjectId::null(gix::hash::Kind::Sha1),
            next_run: stored_next_run,
        });
        assert!(task.state().next_run < stored_next_run);
    }
}
