use std::sync::{Arc, Mutex};

use gix::{hash::Kind, ObjectId};
use kitops::{
    config::GitTaskConfig,
    errors::GitOpsError,
    gix::DefaultUrlProvider,
    receiver::{SourceType, WorkloadEvent},
    workload::{GitWorkload, Workload},
};
use utils::*;

mod utils;

fn config(upstream: &tempfile::TempDir, entrypoint: &str, args: &[&str]) -> GitTaskConfig {
    serde_yaml::from_str(&format!(
        r#"
name: ze-task
git:
    url: file://{}
actions:
    - name: ze-action
      entrypoint: {}
      args: {}
"#,
        upstream.path().to_str().unwrap(),
        entrypoint,
        serde_json::to_string(args).unwrap()
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
    let config = config(&upstream, "/bin/ls", &[]);
    let provider = DefaultUrlProvider::new(config.git.url.clone());
    let mut workload = GitWorkload::new(config, provider, &repodir.path());
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
    let config = config(&upstream, "/usr/bin/false", &[]);
    let provider = DefaultUrlProvider::new(config.git.url.clone());
    let mut workload = GitWorkload::new(config, provider, &repodir.path());
    let events = Arc::new(Mutex::new(Vec::new()));
    let events2 = events.clone();
    workload.watch(move |event| {
        events2.lock().unwrap().push(event);
        Ok(())
    });
    let prev_sha = ObjectId::empty_tree(Kind::Sha1);
    let res = workload.perform(workdir.into_path(), prev_sha);
    assert!(matches!(res, Err(GitOpsError::ActionFailed(..))));
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
    let config = config(&upstream, "/no/such/file", &[]);
    let provider = DefaultUrlProvider::new(config.git.url.clone());
    let mut workload = GitWorkload::new(config, provider, &repodir.path());
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

#[cfg(unix)]
#[test]
fn woarkload_gets_sha_env() {
    let sh = shell();
    let upstream = empty_repo(&sh);
    let next_sha = commit_file(&upstream, "revision 1");
    let repodir = tempfile::tempdir().unwrap();
    let next_sha = ObjectId::from_hex(next_sha.as_bytes()).unwrap();
    let workdir = tempfile::tempdir().unwrap();
    let config = config(&upstream, "/bin/sh", &["-c", "echo $KITOPS_SHA"]);
    let provider = DefaultUrlProvider::new(config.git.url.clone());
    let mut workload = GitWorkload::new(config, provider, &repodir.path());
    let events = Arc::new(Mutex::new(Vec::new()));
    let events2 = events.clone();
    workload.watch(move |event| {
        events2.lock().unwrap().push(event);
        Ok(())
    });
    let prev_sha = ObjectId::empty_tree(Kind::Sha1);
    workload.perform(workdir.into_path(), prev_sha).unwrap();
    assert_eq!(
        events
            .lock()
            .unwrap()
            .iter()
            .find(|e| matches!(e, WorkloadEvent::ActionOutput(..))),
        Some(&WorkloadEvent::ActionOutput(
            "ze-task|ze-action".to_string(),
            SourceType::StdOut,
            format!("{}\n", next_sha).into_bytes(),
        ))
    );
}
