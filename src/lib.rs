use std::{thread::sleep, time::Duration};

use crate::{task::ScheduledTask, workload::Workload};

pub mod actions;
pub mod config;
pub mod errors;
pub mod github;
pub mod gix;
pub mod opts;
pub mod receiver;
pub mod state;
pub mod store;
pub mod task;
#[cfg(test)]
pub(crate) mod testutils;
pub(crate) mod utils;
pub mod workload;

#[derive(Debug, PartialEq)]
pub enum Progress {
    Running,
    Idle,
}

pub fn run_tasks<F, W>(
    tasks: &mut [ScheduledTask<W>],
    mut persist: F,
    once_only: bool,
    poll_interval: Duration,
) -> Result<(), errors::GitOpsError>
where
    F: FnMut(&ScheduledTask<W>) -> Result<(), errors::GitOpsError>,
    W: Workload + Clone + Send + 'static,
{
    loop {
        let res = progress_one_task(tasks, &mut persist)?;
        if res == Progress::Idle {
            if once_only {
                // TODO: We should remove tasks from the list? Current strategy will
                // run continuously if task execution time > task interval.
                return Ok(());
            } else {
                sleep(poll_interval);
            }
        }
    }
}

fn progress_one_task<F, W>(
    tasks: &mut [ScheduledTask<W>],
    persist: &mut F,
) -> Result<Progress, errors::GitOpsError>
where
    F: FnMut(&ScheduledTask<W>) -> Result<(), errors::GitOpsError>,
    W: Workload + Clone + Send + 'static,
{
    if let Some(task) = tasks.iter_mut().find(|t| t.is_eligible()) {
        task.start()?;
        task.schedule_next();
        persist(task)?;
        return Ok(Progress::Running);
    } else if let Some(task) = tasks.iter_mut().find(|t| t.is_finished()) {
        match task.finalize() {
            Ok(_) => persist(task)?,
            Err(err) if err.is_fatal() => return Err(err),
            Err(_) => (),
        }
        return Ok(Progress::Running);
    } else if tasks.iter().any(|t| t.is_running()) {
        return Ok(Progress::Running);
    }
    Ok(Progress::Idle)
}

#[cfg(test)]
mod lib {
    use std::time::{Duration, SystemTime};

    use gix::{hash::Kind, ObjectId};

    use crate::{errors::GitOpsError, state::State, task::ScheduledTask, testutils::TestWorkload};

    #[test]
    fn run_eligible_task() {
        let mut tasks = vec![ScheduledTask::new(TestWorkload::default())];
        let mut persist = |_t: &ScheduledTask<TestWorkload>| Ok(());
        let progress = super::progress_one_task(&mut tasks[..], &mut persist).unwrap();
        assert!(progress == super::Progress::Running);
        assert!(tasks[0].is_running());
        tasks[0].await_finished();
        let progress = super::progress_one_task(&mut tasks[..], &mut persist).unwrap();
        assert!(progress == super::Progress::Running);
        assert!(!tasks[0].is_finished());
        let progress = super::progress_one_task(&mut tasks[..], &mut persist).unwrap();
        assert!(progress == super::Progress::Idle);
    }

    #[test]
    fn dont_start_ineligible_task() {
        let mut tasks = vec![ScheduledTask::new(TestWorkload::default())];
        tasks[0].set_state(State {
            current_sha: ObjectId::empty_blob(Kind::Sha1),
            next_run: SystemTime::now() + Duration::from_secs(1),
        });
        let mut persist = |_t: &ScheduledTask<TestWorkload>| Ok(());
        let progress = super::progress_one_task(&mut tasks[..], &mut persist).unwrap();
        assert!(progress == super::Progress::Idle);
    }

    #[test]
    fn dont_pesist_failing_task() {
        let mut tasks = vec![ScheduledTask::new(TestWorkload::fail_with(|| {
            GitOpsError::ActionFailed("ze-task".to_owned(), "ze-action".to_owned())
        }))];
        let mut persist = |_t: &ScheduledTask<TestWorkload>| Ok(());
        super::progress_one_task(&mut tasks[..], &mut persist).unwrap();
        tasks[0].await_finished();
        super::progress_one_task(&mut tasks[..], &mut persist).unwrap();
        assert_eq!(tasks[0].state().current_sha, ObjectId::null(Kind::Sha1));
    }
}
