#![allow(clippy::module_name_repetitions)]

use clap::Parser;
use errors::GitOpsError;
use opts::{load_store, load_tasks, CliOptions};
use receiver::{logging_receiver, ActionOutput};
use std::{
    collections::HashSet,
    convert::Infallible,
    sync::mpsc::{channel, Sender},
    thread::{sleep, spawn},
    time::Duration,
};
use store::Store;
use task::Task;

mod actions;
mod errors;
mod git;
mod opts;
mod receiver;
mod store;
mod task;

fn run<F>(
    tasks: &mut [Task],
    mut persist: F,
    tx: &Sender<ActionOutput>,
) -> Result<Infallible, GitOpsError>
where
    F: FnMut(&Task) -> Result<(), GitOpsError>,
{
    loop {
        if let Some(task) = tasks.iter_mut().find(|t| t.is_eligible()) {
            let task_tx = tx.clone();
            task.start(task_tx)?;
            task.schedule_next();
            persist(task)?;
            continue;
        }
        if let Some(task) = tasks.iter_mut().find(|t| t.is_finished()) {
            match task.take_result() {
                Ok(new_sha) => {
                    task.processed_sha(new_sha);
                    persist(task)?;
                }
                Err(err) if err.is_fatal() => return Err(err),
                Err(_) => (),
            }
            continue;
        }
        sleep(Duration::from_secs(1));
    }
}

fn main() -> Result<Infallible, GitOpsError> {
    let opts = CliOptions::parse();
    let (tx, rx) = channel();
    // TODO Handle TERM both here and when running actions
    spawn(move || {
        logging_receiver(&rx);
    });
    let mut tasks = load_tasks(&opts)?;
    let mut store = load_store(&opts)?;
    let task_ids = tasks.iter().map(Task::id).collect::<HashSet<_>>();
    store.retain(task_ids);
    for task in &mut tasks {
        if let Some(s) = store.get(&task.id()) {
            task.state = s.clone();
        }
    }
    run(&mut tasks, |t: &Task| store.persist(t), &tx)
}
