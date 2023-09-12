use std::{
    ops::Add,
    path::PathBuf,
    sync::mpsc::Sender,
    thread::{spawn, JoinHandle},
    time::{Duration, Instant, SystemTime},
};

use gix::{hash::Kind, ObjectId, Url};
use serde::{Deserialize, Serialize};

use crate::{
    actions::{run_action, Action},
    errors::GitOpsError,
    git::{ensure_worktree, GitConfig},
    opts::CliOptions,
    receiver::ActionOutput,
};

pub trait Task {
    fn id(&self) -> String;
    fn is_eligible(&self) -> bool;
    fn is_running(&self) -> bool;
    fn is_finished(&self) -> bool;
    fn schedule_next(&mut self);
    fn start(&mut self, tx: Sender<ActionOutput>) -> Result<(), GitOpsError>;
    fn finalize(&mut self) -> Result<(), GitOpsError>;
    fn state(&self) -> State;
    fn set_state(&mut self, state: State);
}

pub struct GitTask {
    config: GitOpsConfig,
    repo_dir: PathBuf,
    pub state: State,
    worker: Option<JoinHandle<Result<ObjectId, GitOpsError>>>,
}

impl GitTask {
    pub fn from_config(config: GitOpsConfig, opts: &CliOptions) -> Self {
        let repo_dir = opts
            .repo_dir
            .as_ref()
            .map(|dir| dir.join(config.git.safe_url()))
            .unwrap();
        GitTask {
            config,
            repo_dir,
            state: State::default(),
            worker: None,
        }
    }
}

impl Task for GitTask {
    fn id(&self) -> String {
        self.config.name.clone()
    }

    fn is_eligible(&self) -> bool {
        self.worker.is_none() && self.state.next_run < SystemTime::now()
    }

    fn is_running(&self) -> bool {
        self.worker.as_ref().is_some_and(|h| !h.is_finished())
    }

    fn is_finished(&self) -> bool {
        self.worker.as_ref().is_some_and(|h| h.is_finished())
    }

    fn schedule_next(&mut self) {
        self.state.next_run = SystemTime::now().add(self.config.interval);
    }

    fn start(&mut self, tx: Sender<ActionOutput>) -> Result<(), GitOpsError> {
        let task_id = self.id();
        let config = self.config.clone();
        let current_sha = self.state.current_sha;
        let repodir = self.repo_dir.clone();
        let workdir = tempfile::tempdir()
            .map_err(GitOpsError::WorkDir)?
            .into_path();
        let deadline = Instant::now() + config.timeout;
        let worker = spawn(move || {
            let new_sha = ensure_worktree(&config.git, deadline, &repodir, &workdir)?;
            if current_sha != new_sha {
                tx.send(ActionOutput::Changes(
                    config.name.clone(),
                    current_sha,
                    new_sha,
                ))
                .map_err(|err| GitOpsError::SendError(format!("{}", err)))?;
                for action in config.actions {
                    let name = format!("{}|{}", task_id, action.id());
                    run_action(&name, &action, &workdir, deadline, &tx)?;
                }
            }
            std::fs::remove_dir_all(workdir).map_err(GitOpsError::WorkDir)?;
            Ok(new_sha)
        });
        self.worker = Some(worker);
        Ok(())
    }

    fn finalize(&mut self) -> Result<(), GitOpsError> {
        let new_sha = self
            .worker
            .take()
            .expect("result only called once")
            .join()
            .unwrap()?;
        self.state.current_sha = new_sha;
        Ok(())
    }

    fn state(&self) -> State {
        self.state.clone()
    }

    fn set_state(&mut self, state: State) {
        self.state = state;
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct State {
    pub next_run: SystemTime,
    pub current_sha: ObjectId,
}

impl Default for State {
    fn default() -> Self {
        Self {
            current_sha: ObjectId::null(Kind::Sha1),
            next_run: SystemTime::now(),
        }
    }
}

#[derive(Clone, Deserialize)]
pub struct GitOpsConfig {
    name: String,
    git: GitConfig,
    actions: Vec<Action>,
    #[serde(default = "GitOpsConfig::default_interval")]
    interval: Duration,
    #[serde(default = "GitOpsConfig::default_timeout")]
    timeout: Duration,
}

impl GitOpsConfig {
    pub fn add_action(&mut self, action: Action) {
        self.actions.push(action);
    }
}

impl TryFrom<&CliOptions> for GitOpsConfig {
    type Error = GitOpsError;

    fn try_from(opts: &CliOptions) -> Result<Self, Self::Error> {
        let url = Url::try_from(opts.url.clone().unwrap()).map_err(GitOpsError::InvalidUrl)?;
        Ok(Self {
            name: url.path.to_string(),
            git: TryFrom::try_from(opts)?,
            actions: Vec::new(),
            interval: opts
                .interval
                .map_or(GitOpsConfig::default_interval(), Duration::from_secs_f32),
            timeout: opts
                .timeout
                .map_or(GitOpsConfig::default_timeout(), Duration::from_secs_f32),
        })
    }
}

impl GitOpsConfig {
    pub fn default_interval() -> Duration {
        Duration::from_secs(60)
    }

    pub fn default_timeout() -> Duration {
        Duration::from_secs(3600)
    }
}
