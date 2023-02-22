use std::{
    ops::Add,
    sync::mpsc::Sender,
    thread::{spawn, JoinHandle},
    time::{Duration, Instant, SystemTime},
};

use gix::{hash::Kind, ObjectId, Url};
use serde::{Deserialize, Serialize};

use crate::{
    actions::{run_action, Action},
    errors::GitOpsError,
    git::{fetch_repo, GitConfig},
    opts::CliOptions,
    receiver::ActionOutput,
};

pub struct Task {
    config: GitOpsConfig,
    pub state: State,
    worker: Option<JoinHandle<Result<ObjectId, GitOpsError>>>,
}

impl Task {
    pub fn from_config(config: GitOpsConfig) -> Self {
        Task {
            config,
            state: State::default(),
            worker: None,
        }
    }
    pub fn id(&self) -> String {
        self.config.name.clone()
    }

    pub fn is_eligible(&self) -> bool {
        self.worker.is_none() && self.state.next_run < SystemTime::now()
    }

    pub fn is_finished(&self) -> bool {
        self.worker.as_ref().map(JoinHandle::is_finished).is_some()
    }

    pub fn processed_sha(&mut self, new_sha: ObjectId) {
        self.state.current_sha = new_sha;
    }

    pub fn schedule_next(&mut self) {
        self.state.next_run = SystemTime::now().add(self.config.interval);
    }

    pub fn start(&mut self, tx: Sender<ActionOutput>) -> Result<(), GitOpsError> {
        let task_id = self.id();
        let config = self.config.clone();
        let current_sha = self.state.current_sha;
        let workdir = tempfile::tempdir()
            .map_err(GitOpsError::WorkDir)?
            .into_path();
        let deadline = Instant::now() + config.timeout;
        let worker = spawn(move || {
            let new_sha = fetch_repo(config.git, deadline, &workdir)?;
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

    pub fn take_result(&mut self) -> Result<ObjectId, GitOpsError> {
        self.worker
            .take()
            .expect("result only called once")
            .join()
            .unwrap()
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
