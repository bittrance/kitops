#![allow(clippy::module_name_repetitions)]

use clap::Parser;
use kitops::errors::GitOpsError;
use kitops::opts::{load_store, load_tasks, CliOptions};
use kitops::receiver::logging_receiver;
use kitops::run_tasks;
use kitops::store::Store;
use kitops::task::gixworkload::GitWorkload;
use kitops::task::scheduled::ScheduledTask;
use std::time::Duration;
use std::{collections::HashSet, sync::mpsc::channel, thread::spawn};

fn main() -> Result<(), GitOpsError> {
    let mut opts = CliOptions::parse();
    opts.complete()?;
    let (tx, rx) = channel();
    // TODO Handle TERM both here and when running actions
    spawn(move || {
        logging_receiver(&rx);
    });
    let mut tasks = load_tasks(&opts)?;
    let mut store = load_store(&opts)?;
    let task_ids = tasks.iter().map(ScheduledTask::id).collect::<HashSet<_>>();
    store.retain(task_ids);
    for task in &mut tasks {
        if let Some(s) = store.get(&task.id()) {
            task.set_state(s.clone());
        }
    }
    run_tasks(
        &mut tasks[..],
        |t: &ScheduledTask<GitWorkload>| store.persist(t.id(), t),
        &tx,
        opts.once_only,
        Duration::from_secs(1),
    )
}
