use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use gix::ObjectId;

use crate::{
    actions::run_action, errors::GitOpsError, git::ensure_worktree, opts::CliOptions,
    receiver::ActionOutput,
};

use super::{
    github::{update_commit_status, GitHubStatus},
    GitTaskConfig, Workload,
};

#[derive(Clone)]
pub struct GitWorkload {
    config: GitTaskConfig,
    repo_dir: PathBuf,
}

impl GitWorkload {
    pub fn from_config(config: GitTaskConfig, opts: &CliOptions) -> Self {
        let repo_dir = opts
            .repo_dir
            .as_ref()
            .map(|dir| dir.join(config.git.safe_url()))
            .unwrap();
        GitWorkload { config, repo_dir }
    }
}

impl Workload for GitWorkload {
    fn id(&self) -> String {
        self.config.name.clone()
    }

    fn interval(&self) -> Duration {
        self.config.interval
    }

    fn work<F>(
        &self,
        workdir: PathBuf,
        current_sha: ObjectId,
        sink: F,
    ) -> Result<ObjectId, GitOpsError>
    where
        F: Fn(ActionOutput) -> Result<(), GitOpsError> + Clone + Send + 'static,
    {
        let config = self.config.clone();
        let task_id = config.name.clone();
        let repodir = self.repo_dir.clone();
        let deadline = Instant::now() + config.timeout;

        let new_sha = ensure_worktree(&config.git, deadline, &repodir, &workdir)?;
        if current_sha != new_sha {
            sink(ActionOutput::Changes(
                config.name.clone(),
                current_sha,
                new_sha,
            ))
            .map_err(|err| GitOpsError::SendError(format!("{}", err)))?;
            for action in config.actions {
                let name = format!("{}|{}", task_id, action.id());
                run_action(&name, &action, &workdir, deadline, &sink)?;
            }
        }
        if let Some(cfg) = config.notify {
            update_commit_status(&cfg, &new_sha.to_string(), GitHubStatus::Success, "Did it")?;
        }
        std::fs::remove_dir_all(&workdir).map_err(GitOpsError::WorkDir)?;
        Ok(new_sha)
    }
}
