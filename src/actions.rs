use std::{
    collections::HashMap,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::mpsc::Sender,
    thread::{sleep, spawn, JoinHandle},
    time::{Duration, Instant},
};

use serde::Deserialize;

use crate::{
    errors::GitOpsError,
    opts::CliOptions,
    receiver::{ActionOutput, SourceType},
};

#[derive(Clone, Deserialize)]
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

fn emit_data<R>(
    name: String,
    mut source: R,
    source_type: SourceType,
    tx: Sender<ActionOutput>,
) -> JoinHandle<Result<(), GitOpsError>>
where
    R: Read + Send + 'static,
{
    spawn(move || {
        let mut buf: [u8; 4096] = [0; 4096];
        while source.read(&mut buf).map_err(GitOpsError::ActionError)? > 0 {
            tx.send(ActionOutput::Output(name.clone(), source_type, buf.into()))
                .map_err(|err| GitOpsError::SendError(format!("{}", err)))?;
        }
        Ok::<(), GitOpsError>(())
    })
}

pub fn run_action(
    name: &str,
    action: &Action,
    cwd: &Path,
    deadline: Instant,
    tx: &Sender<ActionOutput>,
) -> Result<(), GitOpsError> {
    let mut command = build_command(action, cwd);
    let mut child = command.spawn().map_err(GitOpsError::ActionError)?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    // TODO Proper cleanup; break read threads, et c
    emit_data(name.to_string(), stdout, SourceType::StdOut, tx.clone());
    emit_data(name.to_string(), stderr, SourceType::StdErr, tx.clone());
    loop {
        if let Some(exit) = child.try_wait().map_err(GitOpsError::ActionError)? {
            tx.send(ActionOutput::Exit(name.to_string(), exit))
                .map_err(|err| GitOpsError::SendError(format!("{}", err)))?;
            break;
        }
        if Instant::now() > deadline {
            child.kill().map_err(GitOpsError::ActionError)?;
            tx.send(ActionOutput::Timeout(name.to_string()))
                .map_err(|err| GitOpsError::SendError(format!("{}", err)))?;
            break;
        }
        sleep(Duration::from_secs(1));
    }
    Ok(())
}
