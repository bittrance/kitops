use std::sync::{Arc, Mutex};

use clap::Parser;
use gix::{hash::Kind, ObjectId};
use kitops::{
    opts::CliOptions,
    receiver::ActionOutput,
    task::{gixworkload::GitWorkload, GitTaskConfig, Workload}, errors::GitOpsError,
};
use utils::*;

mod utils;

fn cli_options(repodir: &tempfile::TempDir) -> CliOptions {
    CliOptions::parse_from(&[
        "kitops",
        "--repo-dir",
        &repodir.path().to_str().unwrap(),
    ])
}

fn config(upstream: &tempfile::TempDir, entrypoint: &str) -> GitTaskConfig {
    serde_yaml::from_str(&format!(
       r#"
name: ze-task
git:
    url: file://{}
actions:
    - name: ze-action
      entrypoint: {}
"#,
    upstream.path().to_str().unwrap(), entrypoint))
    .unwrap()
}

fn non_action_events(events: Arc<Mutex<Vec<ActionOutput>>>) -> Vec<ActionOutput> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| !matches!(e, ActionOutput::Output(..) | ActionOutput::Exit(..)))
        .cloned()
        .collect::<Vec<_>>()
}

#[test]
fn watch_successful_workload() {
    let sh = shell();
    let upstream = empty_repo(&sh);
    let next_sha = commit_file(&upstream, "revision 1");
    let repodir = tempfile::tempdir().unwrap();
    let next_sha = ObjectId::from_hex(next_sha.as_bytes()).unwrap();
    let workdir = tempfile::tempdir().unwrap();
    let opts = cli_options(&repodir);
    let config = config(&upstream, "/bin/ls");
    let mut workload = GitWorkload::from_config(config, &opts);
    let events = Arc::new(Mutex::new(Vec::new()));
    let events2 = events.clone();
    workload.watch(move |event| {
        events2.lock().unwrap().push(event);
        Ok(())
    });
    let prev_sha = ObjectId::empty_tree(Kind::Sha1);
    workload.work(workdir.into_path(), prev_sha).unwrap();
    assert_eq!(
        non_action_events(events),
        vec![
            ActionOutput::Changes("ze-task".to_string(), prev_sha.clone(), next_sha),
            ActionOutput::Success("ze-task".to_string(), next_sha),
        ]
    );
}

#[test]
fn watch_failing_workload() {
    let sh = shell();
    let upstream = empty_repo(&sh);
    commit_file(&upstream, "revision 1");
    let repodir = tempfile::tempdir().unwrap();
    let workdir = tempfile::tempdir().unwrap();
    let opts = cli_options(&repodir);
    let config = config(&upstream, "/bin/false");
    let mut workload = GitWorkload::from_config(config, &opts);
    let events = Arc::new(Mutex::new(Vec::new()));
    let events2 = events.clone();
    workload.watch(move |event| {
        events2.lock().unwrap().push(event);
        Ok(())
    });
    let prev_sha = ObjectId::empty_tree(Kind::Sha1);
    workload.work(workdir.into_path(), prev_sha).unwrap();
    let events = non_action_events(events);
    assert_eq!(events.len(), 2);
    assert!(matches!(events[0], ActionOutput::Changes(..)));
    assert!(matches!(events[1], ActionOutput::Failure(..)));
}

#[test]
fn watch_erroring_workload() {
    let sh = shell();
    let upstream = empty_repo(&sh);
    commit_file(&upstream, "revision 1");
    let repodir = tempfile::tempdir().unwrap();
    let workdir = tempfile::tempdir().unwrap();
    let opts = cli_options(&repodir);
    let config = config(&upstream, "/no/such/file");
    let mut workload = GitWorkload::from_config(config, &opts);
    let events = Arc::new(Mutex::new(Vec::new()));
    let events2 = events.clone();
    workload.watch(move |event| {
        events2.lock().unwrap().push(event);
        Ok(())
    });
    let prev_sha = ObjectId::empty_tree(Kind::Sha1);
    let res = workload.work(workdir.into_path(), prev_sha);
    assert!(matches!(res, Err(GitOpsError::ActionError(..))));
    let events = non_action_events(events);
    assert_eq!(events.len(), 2);
    assert!(matches!(events[0], ActionOutput::Changes(..)));
    assert!(matches!(events[1], ActionOutput::Error(..)));
}
