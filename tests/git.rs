use xshell::cmd;

use std::time::{Duration, Instant};

use kitops::{config::GitConfig, gix::ensure_worktree};

use utils::{clone_repo, commit_file, empty_repo, reset_branch, shell, TEST_CONFIG};

mod utils;

#[test]
fn clone_repo_from_github_https() {
    let sh = shell();
    let config = serde_yaml::from_str::<GitConfig>(TEST_CONFIG).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let repodir = tempfile::tempdir().unwrap();
    let workdir = tempfile::tempdir().unwrap();
    ensure_worktree(config.url, &config.branch, deadline, &repodir, &workdir).unwrap();
    sh.change_dir(&workdir);
    let files = cmd!(sh, "ls").read().unwrap();
    assert!(files.contains("Cargo.toml"));
    sh.change_dir(&repodir);
    let remotes = cmd!(sh, "git remote -v").read().unwrap();
    assert!(remotes.contains("https://github.com/bittrance/kitops"));
}

#[test]
fn fetch_repo_from_file_url() {
    let sh = shell();
    let upstream = empty_repo(&sh);
    let repodir = clone_repo(&sh, &upstream);
    commit_file(&upstream, "revision 1");
    let config =
        serde_yaml::from_str::<GitConfig>(&format!("url: file://{}", upstream.path().display()))
            .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let workdir = tempfile::tempdir().unwrap();
    ensure_worktree(
        config.url.clone(),
        &config.branch,
        deadline,
        &repodir,
        &workdir,
    )
    .unwrap();
    assert_eq!(
        sh.read_file(workdir.path().join("ze-file")).unwrap(),
        "revision 1"
    );
    commit_file(&upstream, "revision 2");
    let workdir = tempfile::tempdir().unwrap();
    ensure_worktree(
        config.url.clone(),
        &config.branch,
        deadline,
        &repodir,
        &workdir,
    )
    .unwrap();
    assert_eq!(
        sh.read_file(workdir.path().join("ze-file")).unwrap(),
        "revision 2"
    );
}

#[test]
fn fetch_repo_with_force_push() {
    let sh = shell();
    let upstream = empty_repo(&sh);
    let sha1 = commit_file(&upstream, "revision 1");
    commit_file(&upstream, "revision 2");
    let repodir = clone_repo(&sh, &upstream);
    reset_branch(&sh, &upstream, &sha1);
    let config =
        serde_yaml::from_str::<GitConfig>(&format!("url: file://{}", upstream.path().display()))
            .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let workdir = tempfile::tempdir().unwrap();
    ensure_worktree(config.url, &config.branch, deadline, &repodir, &workdir).unwrap();
    assert_eq!(
        sh.read_file(workdir.path().join("ze-file")).unwrap(),
        "revision 1"
    );
}
