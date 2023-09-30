#![allow(dead_code)]

use std::path::Path;

use tempfile::TempDir;
use xshell::{cmd, Shell};

pub static TEST_CONFIG: &str = r#"
url: https://github.com/bittrance/kitops
branch: main
"#;

pub fn shell() -> Shell {
    let sh = Shell::new().unwrap();
    sh.set_var("GIT_CONFIG_SYSTEM", "/dev/null");
    sh.set_var("GIT_CONFIG_GLOBAL", "/dev/null");
    sh
}

pub fn empty_repo(sh: &Shell) -> TempDir {
    let upstream = tempfile::tempdir().unwrap();
    let ref pupstream = upstream.path();
    cmd!(sh, "git init -b main {pupstream}")
        .ignore_stdout()
        .run()
        .unwrap();
    upstream
}

pub fn clone_repo<P>(sh: &Shell, source: P) -> TempDir
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

pub fn commit_file<P>(dir: P, content: &str) -> String
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

pub fn reset_branch(sh: &Shell, dir: &TempDir, target: &str) {
    sh.change_dir(dir.path());
    cmd!(sh, "git reset --hard {target}")
        .ignore_stdout()
        .run()
        .unwrap();
}
