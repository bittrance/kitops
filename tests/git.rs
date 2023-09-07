use tempfile::TempDir;
use xshell::{cmd, Shell};

use std::{
    path::Path,
    time::{Duration, Instant},
};

use kitops::git::{ensure_worktree, GitConfig};

static TEST_CONFIG: &str = r#"
url: https://github.com/bittrance/kitops
branch: main
"#;

fn shell() -> Shell {
    let sh = Shell::new().unwrap();
    sh.set_var("GIT_CONFIG_SYSTEM", "/dev/null");
    sh.set_var("GIT_CONFIG_GLOBAL", "/dev/null");
    sh
}

fn empty_repo(sh: &Shell) -> TempDir {
    let upstream = tempfile::tempdir().unwrap();
    let ref pupstream = upstream.path();
    cmd!(sh, "git init -b main {pupstream}")
        .ignore_stdout()
        .run()
        .unwrap();
    upstream
}

fn clone_repo<P>(sh: &Shell, source: P) -> TempDir
where
    P: AsRef<Path>,
{
    let repodir = tempfile::tempdir().unwrap();
    let ref psource = source.as_ref().as_os_str();
    let ref prepodir = repodir.path();
    cmd!(sh, "git clone -q {psource} {prepodir}")
        .ignore_stdout()
        .run()
        .unwrap();
    repodir
}

fn commit_file<P>(dir: P, content: &str) -> String
where
    P: AsRef<Path>,
{
    let dir = dir.as_ref();
    let sh = shell();
    sh.change_dir(dir);
    sh.write_file(dir.join("ze-file"), content).unwrap();
    cmd!(sh, "git add ze-file").ignore_stdout().run().unwrap();
    cmd!(sh, "git -c user.email=testing@example.com -c user.name=Testing commit -m 'Committing {content}'").ignore_stdout().run().unwrap();
    cmd!(sh, "git rev-parse HEAD").read().unwrap()
}

fn reset_branch(sh: &Shell, dir: &TempDir, target: &str) {
    sh.change_dir(dir.path());
    cmd!(sh, "git reset --hard {target}")
        .ignore_stdout()
        .run()
        .unwrap();
}

#[test]
fn clone_repo_from_github_https() {
    let sh = shell();
    let config = serde_yaml::from_str::<GitConfig>(TEST_CONFIG).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let repodir = tempfile::tempdir().unwrap();
    let workdir = tempfile::tempdir().unwrap();
    ensure_worktree(&config, deadline, &repodir, &workdir).unwrap();
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
    let config = serde_yaml::from_str::<GitConfig>(&format!(
        "url: file://{}\nbranch: main",
        upstream.path().display()
    ))
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let workdir = tempfile::tempdir().unwrap();
    ensure_worktree(&config, deadline, &repodir, &workdir).unwrap();
    assert_eq!(
        sh.read_file(workdir.path().join("ze-file")).unwrap(),
        "revision 1"
    );
    commit_file(&upstream, "revision 2");
    let workdir = tempfile::tempdir().unwrap();
    ensure_worktree(&config, deadline, &repodir, &workdir).unwrap();
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
    let config = serde_yaml::from_str::<GitConfig>(&format!(
        "url: file://{}\nbranch: main",
        upstream.path().display()
    ))
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let workdir = tempfile::tempdir().unwrap();
    ensure_worktree(&config, deadline, &repodir, &workdir).unwrap();
    assert_eq!(
        sh.read_file(workdir.path().join("ze-file")).unwrap(),
        "revision 1"
    );
}
