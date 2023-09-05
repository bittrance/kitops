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

fn commit_file<P>(dir: P, content: &str)
where
    P: AsRef<Path>,
{
    let dir = dir.as_ref();
    let sh = Shell::new().unwrap();
    sh.set_var("GIT_CONFIG_SYSTEM", "/dev/null");
    sh.set_var("GIT_CONFIG_GLOBAL", "/dev/null");
    sh.change_dir(dir);
    sh.write_file(dir.join("ze-file"), content).unwrap();
    cmd!(sh, "git add ze-file").ignore_stdout().run().unwrap();
    cmd!(sh, "git -c user.email=testing@example.com -c user.name=Testing commit -m 'Committing {content}'").ignore_stdout().run().unwrap();
}

#[test]
fn clone_repo_from_github_https() {
    let config = serde_yaml::from_str::<GitConfig>(TEST_CONFIG).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let repodir = tempfile::tempdir().unwrap();
    let workdir = tempfile::tempdir().unwrap();
    ensure_worktree(
        &config,
        deadline,
        &repodir.path().join("test-repo"),
        &workdir.path(),
    )
    .unwrap();
    let sh = Shell::new().unwrap();
    sh.set_var("GIT_CONFIG_SYSTEM", "/dev/null");
    sh.set_var("GIT_CONFIG_GLOBAL", "/dev/null");
    sh.change_dir(&workdir);
    let files = cmd!(sh, "ls").read().unwrap();
    assert!(files.contains("Cargo.toml"));
    sh.change_dir(repodir.path().join("test-repo"));
    let remotes = cmd!(sh, "git remote -v").read().unwrap();
    assert!(remotes.contains("https://github.com/bittrance/kitops"));
}

#[test]
fn fetch_repo_from_file_url() {
    let sh = Shell::new().unwrap();
    sh.set_var("GIT_CONFIG_SYSTEM", "/dev/null");
    sh.set_var("GIT_CONFIG_GLOBAL", "/dev/null");
    let upstream = tempfile::tempdir().unwrap();
    let repodir = tempfile::tempdir().unwrap();
    let ref pupstream = upstream.path();
    let ref prepodir = repodir.path();
    sh.change_dir(&upstream);
    cmd!(sh, "git init -b main").ignore_stdout().run().unwrap();
    sh.change_dir(&repodir);
    cmd!(sh, "git clone -q {pupstream} {prepodir}")
        .ignore_stdout()
        .run()
        .unwrap();
    commit_file(&upstream, "stuff");

    let config = serde_yaml::from_str::<GitConfig>(&format!(
        "url: file://{}\nbranch: main",
        pupstream.display()
    ))
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let workdir = tempfile::tempdir().unwrap();
    assert!(!sh.path_exists(workdir.path().join("ze-file")));
    ensure_worktree(&config, deadline, repodir.path(), workdir.path()).unwrap();
    assert!(sh.path_exists(workdir.path().join("ze-file")));
}
