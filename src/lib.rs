use std::{sync::mpsc::Sender, thread::sleep, time::Duration};

pub mod actions;
pub mod errors;
pub mod git;
pub mod opts;
pub mod receiver;
pub mod store;
pub mod task;

#[derive(Debug, PartialEq)]
pub enum Progress {
    Running,
    Idle,
}

pub fn run_tasks<F, T>(
    tasks: &mut [T],
    mut persist: F,
    tx: &Sender<receiver::ActionOutput>,
    once_only: bool,
    poll_interval: Duration,
) -> Result<(), errors::GitOpsError>
where
    F: FnMut(&T) -> Result<(), errors::GitOpsError>,
    T: task::Task,
{
    loop {
        let res = progress_one_task(tasks, &mut persist, tx)?;
        if (res == Progress::Idle) && once_only {
            return Ok(());
        }
        sleep(poll_interval);
    }
}

fn progress_one_task<F, T>(
    tasks: &mut [T],
    persist: &mut F,
    tx: &Sender<receiver::ActionOutput>,
) -> Result<Progress, errors::GitOpsError>
where
    F: FnMut(&T) -> Result<(), errors::GitOpsError>,
    T: task::Task,
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
    use std::cell::RefCell;
    use std::sync::mpsc::Sender;

    use crate::errors::GitOpsError;
    use crate::receiver::ActionOutput;
    use crate::task::{State, Task};

    #[derive(Default)]
    struct TestTask {
        pub id: String,
        pub status: RefCell<Option<bool>>,
        pub eligible: RefCell<bool>,
    }

    impl TestTask {
        pub fn new(id: String) -> Self {
            Self {
                id,
                ..Default::default()
            }
        }
        pub fn make_eligible(&self) {
            *self.eligible.borrow_mut() = true;
        }

        pub fn run(&self) {
            *self.eligible.borrow_mut() = false;
            *self.status.borrow_mut() = Some(true);
        }

        pub fn complete(&self) {
            let mut v = self.status.borrow_mut();
            if v.is_some() {
                *v = Some(false);
            }
        }
    }

    impl Task for TestTask {
        fn id(&self) -> String {
            self.id.clone()
        }

        fn is_eligible(&self) -> bool {
            self.status.borrow().is_none() && *self.eligible.borrow()
        }

        fn is_running(&self) -> bool {
            self.status.borrow().is_some_and(|s| s)
        }

        fn is_finished(&self) -> bool {
            self.status.borrow().is_some_and(|s| !s)
        }

        fn schedule_next(&mut self) {}

        fn start(&mut self, _: Sender<ActionOutput>) -> Result<(), GitOpsError> {
            assert!(*self.eligible.borrow());
            assert!(self.status.borrow().is_none());
            self.run();
            Ok(())
        }

        fn finalize(&mut self) -> Result<(), GitOpsError> {
            assert!(!*self.eligible.borrow());
            assert!(self.status.borrow().is_some());
            *self.status.borrow_mut() = None;
            Ok(())
        }

        fn state(&self) -> State {
            todo!("Not needed")
        }

        fn set_state(&mut self, _: State) {
            todo!("Not needed")
        }
    }

    #[test]
    fn dont_start_ineligible_task() {
        let mut tasks = vec![TestTask::new("id-1".to_owned())];
        let (tx, _) = std::sync::mpsc::channel();
        let mut persist = |_t: &TestTask| Ok(());
        let progress = super::progress_one_task(&mut tasks[..], &mut persist, &tx).unwrap();
        assert!(progress == super::Progress::Idle);
        assert!(!tasks[0].is_running());
    }

    #[test]
    fn run_eligible_task() {
        let mut tasks = vec![TestTask::new("id-1".to_owned())];
        let (tx, _) = std::sync::mpsc::channel();
        let mut persist = |_t: &TestTask| Ok(());
        tasks[0].make_eligible();
        let progress = super::progress_one_task(&mut tasks[..], &mut persist, &tx).unwrap();
        assert!(progress == super::Progress::Running);
        assert!(tasks[0].is_running());
        tasks[0].complete();
        let progress = super::progress_one_task(&mut tasks[..], &mut persist, &tx).unwrap();
        assert!(progress == super::Progress::Running);
        assert!(!tasks[0].is_eligible());
    }
}
