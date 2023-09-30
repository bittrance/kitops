use std::sync::{Arc, Mutex};

use clap::Parser;
use gix::{hash::Kind, ObjectId};
use kitops::{
    opts::CliOptions,
    receiver::ActionOutput,
    task::{gixworkload::GitWorkload, GitTaskConfig, Workload},
};
use utils::*;

mod utils;

#[test]
fn watch_workload() {
    let sh = shell();
    let upstream = empty_repo(&sh);
    let next_sha = commit_file(&upstream, "revision 1");
    let next_sha = ObjectId::from_hex(next_sha.as_bytes()).unwrap();
    let repodir = tempfile::tempdir().unwrap();
    let workdir = tempfile::tempdir().unwrap();
    let opts = CliOptions::parse_from(&[
        "kitops",
        "--url",
        &format!("file://{}", &upstream.path().to_str().unwrap()),
        "--branch",
        "main",
        "--action",
        "/bin/ls",
        "--repo-dir",
        &repodir.path().to_str().unwrap(),
    ]);
    let config = GitTaskConfig::try_from(&opts).unwrap();
    let mut workload = GitWorkload::from_config(config, &opts);
    let events = Arc::new(Mutex::new(Vec::new()));
    let events2 = events.clone();
    workload.watch(move |event| {
        events2.lock().unwrap().push(event);
        Ok(())
    });
    let prev_sha = ObjectId::empty_tree(Kind::Sha1);
    workload.work(workdir.into_path(), prev_sha).unwrap();
    let name = upstream.path().to_str().unwrap().to_owned();
    assert_eq!(
        events.lock().unwrap().clone(),
        vec![
            ActionOutput::Changes(name.clone(), prev_sha.clone(), next_sha),
            ActionOutput::Success(name.clone(), next_sha),
        ]
    );
}
