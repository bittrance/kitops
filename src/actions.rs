use std::{
    collections::HashMap,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread::{sleep, spawn, JoinHandle},
    time::Instant,
};

use serde::Deserialize;

use crate::{
    errors::GitOpsError,
    opts::CliOptions,
    receiver::{SourceType, WorkloadEvent},
    utils::POLL_INTERVAL,
};

#[derive(Debug, PartialEq)]
pub enum ActionResult {
    Success,
    Failure,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Action {
    name: String,
    entrypoint: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    environment: HashMap<String, String>,
    #[serde(default)]
    inherit_environment: bool,
}

impl Action {
    pub fn id(&self) -> String {
        self.name.clone()
    }
}

impl TryFrom<&CliOptions> for Action {
    type Error = GitOpsError;

    fn try_from(opts: &CliOptions) -> Result<Self, Self::Error> {
        let mut environment = HashMap::new();
        for env in &opts.environment {
            let (key, val) = env
                .split_once('=')
                .ok_or_else(|| GitOpsError::InvalidEnvVar(env.clone()))?;
            environment.insert(key.to_owned(), val.to_owned());
        }
        Ok(Self {
            name: opts.action.clone().unwrap(),
            // TODO --action won't work on Windows
            entrypoint: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), opts.action.clone().unwrap()],
            environment,
            inherit_environment: false,
        })
    }
}

fn build_command(action: &Action, cwd: &Path) -> Command {
    let mut command = Command::new(action.entrypoint.clone());
    command.args(action.args.clone());
    if !action.inherit_environment {
        command.env_clear();
        if let Ok(path) = std::env::var("PATH") {
            command.env("PATH", path);
        }
    }
    command.envs(action.environment.iter());
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
    let mut command = build_command(action, cwd);
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
        process::ExitStatus,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use super::*;
    use tempfile::tempdir;

    fn shell_action(cmd: &str) -> Action {
        Action {
            name: "test".to_owned(),
            entrypoint: "/bin/sh".to_owned(),
            args: vec!["-c".to_owned(), cmd.to_owned()],
            environment: HashMap::new(),
            inherit_environment: false,
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
