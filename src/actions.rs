use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread::{sleep, spawn, JoinHandle},
    time::Instant,
};

use crate::{
    config::ActionConfig,
    errors::GitOpsError,
    receiver::{SourceType, WorkloadEvent},
    utils::POLL_INTERVAL,
};

#[derive(Debug, PartialEq)]
pub enum ActionResult {
    Success,
    Failure,
}

#[derive(Clone)]
pub struct Action {
    config: ActionConfig,
}

impl Action {
    pub fn new(config: ActionConfig) -> Self {
        Action { config }
    }

    pub fn id(&self) -> String {
        self.config.name.clone()
    }

    pub fn set_env(&mut self, key: String, val: String) {
        self.config.environment.insert(key, val);
    }
}

fn build_command(config: &ActionConfig, cwd: &Path) -> Command {
    let mut command = Command::new(config.entrypoint.clone());
    command.args(config.args.clone());
    if !config.inherit_environment {
        command.env_clear();
        if let Ok(path) = std::env::var("PATH") {
            command.env("PATH", path);
        }
    }
    command.envs(config.environment.iter());
    command.current_dir(cwd);
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command
}

fn emit_data<F, R>(
    name: String,
    mut source: R,
    source_type: SourceType,
    sink: &Arc<Mutex<F>>,
) -> JoinHandle<Result<(), GitOpsError>>
where
    R: Read + Send + 'static,
    F: Fn(WorkloadEvent) -> Result<(), GitOpsError> + Send + 'static,
{
    let sink = Arc::clone(sink);
    spawn(move || {
        let mut buf: [u8; 4096] = [0; 4096];
        loop {
            let len = source.read(&mut buf).map_err(GitOpsError::ActionError)?;
            if len == 0 {
                break;
            }
            sink.lock().unwrap()(WorkloadEvent::ActionOutput(
                name.clone(),
                source_type,
                buf[..len].into(),
            ))?;
        }
        Ok::<(), GitOpsError>(())
    })
}

pub fn run_action<F>(
    name: &str,
    action: &Action,
    cwd: &Path,
    deadline: Instant,
    sink: &Arc<Mutex<F>>,
) -> Result<ActionResult, GitOpsError>
where
    F: Fn(WorkloadEvent) -> Result<(), GitOpsError> + Send + 'static,
{
    let mut command = build_command(&action.config, cwd);
    let mut child = command.spawn().map_err(GitOpsError::ActionError)?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let out_t = emit_data(name.to_string(), stdout, SourceType::StdOut, sink);
    let err_t = emit_data(name.to_string(), stderr, SourceType::StdErr, sink);
    loop {
        if let Some(exit) = child.try_wait().map_err(GitOpsError::ActionError)? {
            out_t.join().unwrap()?;
            err_t.join().unwrap()?;
            sink.lock().unwrap()(WorkloadEvent::ActionExit(name.to_string(), exit))?;
            if exit.success() {
                break Ok(ActionResult::Success);
            } else {
                break Ok(ActionResult::Failure);
            }
        }
        if Instant::now() > deadline {
            child.kill().map_err(GitOpsError::ActionError)?;
            out_t.join().unwrap()?;
            err_t.join().unwrap()?;
            sink.lock().unwrap()(WorkloadEvent::Timeout(name.to_string()))?;
            break Ok(ActionResult::Failure);
        }
        sleep(POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        process::ExitStatus,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use super::*;
    use tempfile::tempdir;

    fn shell_action(cmd: &str) -> Action {
        Action {
            config: ActionConfig {
                name: "test".to_owned(),
                entrypoint: "/bin/sh".to_owned(),
                args: vec!["-c".to_owned(), cmd.to_owned()],
                environment: HashMap::new(),
                inherit_environment: false,
            },
        }
    }

    #[test]
    #[cfg(unix)]
    fn test_run_action() {
        use std::os::unix::process::ExitStatusExt;

        let action = shell_action("echo test");
        let workdir = tempdir().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let events = Arc::new(Mutex::new(Vec::new()));
        let events2 = events.clone();
        let sink = Arc::new(Mutex::new(move |event| {
            events2.lock().unwrap().push(event);
            Ok(())
        }));
        let res = run_action("test", &action, workdir.path(), deadline, &sink);
        assert!(matches!(res, Ok(ActionResult::Success)));
        assert_eq!(
            vec![
                WorkloadEvent::ActionOutput(
                    "test".to_owned(),
                    SourceType::StdOut,
                    b"test\n".to_vec()
                ),
                WorkloadEvent::ActionExit("test".to_owned(), ExitStatus::from_raw(0)),
            ],
            events.lock().unwrap()[..]
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_run_failing_action() {
        let action = shell_action("exit 1");
        let workdir = tempdir().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let sink = Arc::new(Mutex::new(move |_| Ok(())));
        let res = run_action("test", &action, workdir.path(), deadline, &sink);
        assert!(matches!(res, Ok(ActionResult::Failure)));
    }

    #[test]
    #[cfg(unix)]
    fn timing_out_action() {
        let action = shell_action("sleep 1");
        let workdir = tempdir().unwrap();
        let deadline = Instant::now();
        let sink = Arc::new(Mutex::new(move |_| Ok(())));
        let res = run_action("test", &action, workdir.path(), deadline, &sink);
        assert!(matches!(res, Ok(ActionResult::Failure)));
    }
}
