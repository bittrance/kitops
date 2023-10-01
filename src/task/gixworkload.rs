use std::{
    path::{PathBuf, Path},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use gix::ObjectId;

use crate::{
    actions::{run_action, ActionResult},
    errors::GitOpsError,
    git::ensure_worktree,
    opts::CliOptions,
    receiver::ActionOutput,
};

use super::{GitTaskConfig, Workload};

#[allow(clippy::type_complexity)]
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

    fn run_actions(
        &self,
        workdir: &Path,
        deadline: Instant,
        sink: &Arc<Mutex<impl Fn(ActionOutput) -> Result<(), GitOpsError> + Send + 'static>>,
    ) -> Result<Option<String>, GitOpsError> {
        for action in &self.config.actions {
            let name = format!("{}|{}", self.config.name, action.id());
            let res = run_action(&name, action, workdir, deadline, sink)?;
            if res != ActionResult::Success {
                return Ok(Some(name));
            }
        }
        Ok(None)
    }
}

impl Workload for GitWorkload {
    fn id(&self) -> String {
        self.config.name.clone()
    }

    fn interval(&self) -> Duration {
        self.config.interval
    }

    fn work(&self, workdir: PathBuf, current_sha: ObjectId) -> Result<ObjectId, GitOpsError> {
        let deadline = Instant::now() + self.config.timeout;
        let watchers = self.watchers.clone();
        let sink = Arc::new(Mutex::new(move |event: ActionOutput| {
            for watcher in &watchers {
                watcher.lock().unwrap()(event.clone())?;
            }
            Ok::<_, GitOpsError>(())
        }));

        let new_sha = ensure_worktree(&self.config.git, deadline, &self.repo_dir, &workdir)?;
        if current_sha != new_sha {
            sink.lock().unwrap()(ActionOutput::Changes(
                self.config.name.clone(),
                current_sha,
                new_sha,
            ))
            .map_err(|err| GitOpsError::NotifyError(format!("{}", err)))?;
            match self.run_actions(&workdir, deadline, &sink) {
                Ok(None) => {
                    sink.lock().unwrap()(ActionOutput::Success(self.config.name.clone(), new_sha))
                        .map_err(|err| GitOpsError::NotifyError(format!("{}", err)))?
                }
                Ok(Some(action_name)) => {
                    sink.lock().unwrap()(ActionOutput::Failure(
                        self.config.name.clone(),
                        action_name,
                        new_sha,
                    ))
                    .map_err(|err| GitOpsError::NotifyError(format!("{}", err)))?;
                }
                Err(err) => {
                    sink.lock().unwrap()(ActionOutput::Error(
                        self.config.name.clone(),
                        format!("{}", err),
                        new_sha,
                    ))
                    .map_err(|err| GitOpsError::NotifyError(format!("{}", err)))?;
                    return Err(err);
                }
            }
        }
        std::fs::remove_dir_all(&workdir).map_err(GitOpsError::WorkDir)?;
        Ok(new_sha)
    }
}
