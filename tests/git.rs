use xshell::{cmd, Shell};

use std::time::{Duration, Instant};

use kitops::git::{ensure_worktree, GitConfig};

static TEST_CONFIG: &str = r#"
url: https://github.com/bittrance/kitops
branch: main
"#;

#[test]
fn clone_repo_from_github_https() {
    let config = serde_yaml::from_str::<GitConfig>(TEST_CONFIG).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let repodir = tempfile::tempdir().unwrap();
    let workdir = tempfile::tempdir().unwrap();
    ensure_worktree(&config, deadline, &repodir.path().join("test-repo"), &workdir.path()).unwrap();
    let sh = Shell::new().unwrap();
    sh.change_dir(&workdir);
    let files = cmd!(sh, "ls").read().unwrap();
    assert!(files.contains("Cargo.toml"));
    sh.change_dir(repodir.path().join("test-repo"));
    let remotes = cmd!(sh, "git remote -v").read().unwrap();
    assert!(remotes.contains("https://github.com/bittrance/kitops"));
}
