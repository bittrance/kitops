use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use gix::ObjectId;

use crate::{
    actions::run_action, errors::GitOpsError, git::ensure_worktree, opts::CliOptions,
    receiver::ActionOutput,
};

use super::{GitTaskConfig, Workload};

#[derive(Clone)]
pub struct GitWorkload {
    config: GitTaskConfig,
    repo_dir: PathBuf,
    watchers:
        Vec<Arc<Mutex<Box<dyn Fn(ActionOutput) -> Result<(), GitOpsError> + Send + 'static>>>>,
}

impl GitWorkload {
    pub fn from_config(config: GitTaskConfig, opts: &CliOptions) -> Self {
        let repo_dir = opts
            .repo_dir
            .as_ref()
            .map(|dir| dir.join(config.git.safe_url()))
            .unwrap();
        GitWorkload {
            config,
            repo_dir,
            watchers: Vec::new(),
        }
    }

    pub fn watch(
        &mut self,
        watcher: impl Fn(ActionOutput) -> Result<(), GitOpsError> + Send + 'static,
    ) {
        self.watchers.push(Arc::new(Mutex::new(Box::new(watcher))));
    }
}

impl Workload for GitWorkload {
    fn id(&self) -> String {
        self.config.name.clone()
    }

    fn interval(&self) -> Duration {
        self.config.interval
    }

    // TODO: Take ownership of self to skip the clonefest
    fn work(&self, workdir: PathBuf, current_sha: ObjectId) -> Result<ObjectId, GitOpsError> {
        let config = self.config.clone();
        let task_id = config.name.clone();
        let repodir = self.repo_dir.clone();
        let deadline = Instant::now() + config.timeout;
        let watchers = self.watchers.clone();
        let sink = Arc::new(Mutex::new(move |event: ActionOutput| {
            for watcher in &watchers {
                watcher.lock().unwrap()(event.clone())?;
            }
            Ok(())
        }));

        let new_sha = ensure_worktree(&config.git, deadline, &repodir, &workdir)?;
        if current_sha != new_sha {
            sink.lock().unwrap()(ActionOutput::Changes(
                config.name.clone(),
                current_sha,
                new_sha,
            ))
            .map_err(|err| GitOpsError::NotifyError(format!("{}", err)))?;
            for action in config.actions {
                let name = format!("{}|{}", task_id, action.id());
                run_action(&name, &action, &workdir, deadline, &sink)?;
            }
            sink.lock().unwrap()(ActionOutput::Success(config.name.clone(), new_sha))
                .map_err(|err| GitOpsError::NotifyError(format!("{}", err)))?;
        }
        std::fs::remove_dir_all(&workdir).map_err(GitOpsError::WorkDir)?;
        Ok(new_sha)
    }
}
