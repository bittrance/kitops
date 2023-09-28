use std::{sync::mpsc::Sender, thread::sleep, time::Duration};

use task::{scheduled::ScheduledTask, Workload};

pub mod actions;
pub mod errors;
pub mod git;
pub mod opts;
pub mod receiver;
pub mod store;
pub mod task;
#[cfg(test)]
pub(crate) mod testutils;
pub(crate) mod utils;

#[derive(Debug, PartialEq)]
pub enum Progress {
    Running,
    Idle,
}

pub fn run_tasks<F, W>(
    tasks: &mut [ScheduledTask<W>],
    mut persist: F,
    tx: &Sender<receiver::ActionOutput>,
    once_only: bool,
    poll_interval: Duration,
) -> Result<(), errors::GitOpsError>
where
    F: FnMut(&ScheduledTask<W>) -> Result<(), errors::GitOpsError>,
    W: Workload + Clone + Send + 'static,
{
    loop {
        let res = progress_one_task(tasks, &mut persist, tx)?;
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
    tx: &Sender<receiver::ActionOutput>,
) -> Result<Progress, errors::GitOpsError>
where
    F: FnMut(&ScheduledTask<W>) -> Result<(), errors::GitOpsError>,
    W: Workload + Clone + Send + 'static,
{
    if let Some(task) = tasks.iter_mut().find(|t| t.is_eligible()) {
        let task_tx = tx.clone();
        task.start(task_tx)?;
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

    use crate::{
        task::{scheduled::ScheduledTask, State},
        testutils::TestWorkload,
    };

    #[test]
    fn run_eligible_task() {
        let mut tasks = vec![ScheduledTask::new(TestWorkload::default())];
        let (tx, _) = std::sync::mpsc::channel();
        let mut persist = |_t: &ScheduledTask<TestWorkload>| Ok(());
        let progress = super::progress_one_task(&mut tasks[..], &mut persist, &tx).unwrap();
        assert!(progress == super::Progress::Running);
        assert!(tasks[0].is_running());
        tasks[0].await_finished();
        let progress = super::progress_one_task(&mut tasks[..], &mut persist, &tx).unwrap();
        assert!(progress == super::Progress::Running);
        assert!(!tasks[0].is_finished());
        let progress = super::progress_one_task(&mut tasks[..], &mut persist, &tx).unwrap();
        assert!(progress == super::Progress::Idle);
    }

    #[test]
    fn dont_start_ineligible_task() {
        let mut tasks = vec![ScheduledTask::new(TestWorkload::default())];
        tasks[0].set_state(State {
            next_run: SystemTime::now() + Duration::from_secs(1),
            current_sha: Default::default(),
        });
        let (tx, _) = std::sync::mpsc::channel();
        let mut persist = |_t: &ScheduledTask<TestWorkload>| Ok(());
        let progress = super::progress_one_task(&mut tasks[..], &mut persist, &tx).unwrap();
        assert!(progress == super::Progress::Idle);
    }
}
