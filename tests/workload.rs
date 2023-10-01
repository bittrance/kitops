use std::sync::{Arc, Mutex};

use clap::Parser;
use gix::{hash::Kind, ObjectId};
use kitops::{
    errors::GitOpsError,
    opts::CliOptions,
    receiver::WorkloadEvent,
    task::{gixworkload::GitWorkload, GitTaskConfig, Workload},
};
use utils::*;

mod utils;

fn cli_options(repodir: &tempfile::TempDir) -> CliOptions {
    CliOptions::parse_from(&["kitops", "--repo-dir", &repodir.path().to_str().unwrap()])
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
        upstream.path().to_str().unwrap(),
        entrypoint
    ))
    .unwrap()
}

fn non_action_events(events: Arc<Mutex<Vec<WorkloadEvent>>>) -> Vec<WorkloadEvent> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| {
            !matches!(
                e,
                WorkloadEvent::ActionOutput(..) | WorkloadEvent::ActionExit(..)
            )
        })
        .cloned()
        .collect::<Vec<_>>()
}

#[cfg(unix)]
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
    workload.perform(workdir.into_path(), prev_sha).unwrap();
    assert_eq!(
        non_action_events(events),
        vec![
            WorkloadEvent::Changes("ze-task".to_string(), prev_sha.clone(), next_sha),
            WorkloadEvent::Success("ze-task".to_string(), next_sha),
        ]
    );
}

#[cfg(unix)]
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
    workload.perform(workdir.into_path(), prev_sha).unwrap();
    let events = non_action_events(events);
    assert_eq!(events.len(), 2);
    assert!(matches!(events[0], WorkloadEvent::Changes(..)));
    assert!(matches!(events[1], WorkloadEvent::Failure(..)));
}

#[cfg(unix)]
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
    let res = workload.perform(workdir.into_path(), prev_sha);
    assert!(matches!(res, Err(GitOpsError::ActionError(..))));
    let events = non_action_events(events);
    assert_eq!(events.len(), 2);
    assert!(matches!(events[0], WorkloadEvent::Changes(..)));
    assert!(matches!(events[1], WorkloadEvent::Error(..)));
}
