use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use gix::ObjectId;

use crate::{
    actions::{run_action, Action, ActionResult},
    config::GitTaskConfig,
    errors::GitOpsError,
    gix::{ensure_worktree, UrlProvider},
    receiver::WorkloadEvent,
};

pub trait Workload {
    fn id(&self) -> String;
    fn interval(&self) -> Duration;
    fn perform(self, workdir: PathBuf, current_sha: ObjectId) -> Result<ObjectId, GitOpsError>;
}

#[allow(clippy::type_complexity)]
#[derive(Clone)]
pub struct GitWorkload {
    actions: Vec<Action>,
    config: GitTaskConfig,
    url_provider: Arc<Box<dyn UrlProvider>>,
    repo_dir: PathBuf,
    watchers:
        Vec<Arc<Mutex<Box<dyn Fn(WorkloadEvent) -> Result<(), GitOpsError> + Send + 'static>>>>,
}

impl GitWorkload {
    pub fn new(
        config: GitTaskConfig,
        url_provider: impl UrlProvider + 'static,
        repo_dir: &Path,
    ) -> Self {
        let repo_dir = repo_dir.join(url_provider.safe_url());
        let actions = config
            .actions
            .iter()
            .map(|config| Action::new(config.clone()))
            .collect();
        GitWorkload {
            actions,
            config,
            url_provider: Arc::new(Box::new(url_provider)),
            repo_dir,
            watchers: Vec::new(),
        }
    }

    pub fn watch(
        &mut self,
        watcher: impl Fn(WorkloadEvent) -> Result<(), GitOpsError> + Send + 'static,
    ) {
        self.watchers.push(Arc::new(Mutex::new(Box::new(watcher))));
    }

    fn run_actions(
        &self,
        workdir: &Path,
        deadline: Instant,
        sink: &Arc<Mutex<impl Fn(WorkloadEvent) -> Result<(), GitOpsError> + Send + 'static>>,
    ) -> Result<Option<String>, GitOpsError> {
        for action in &self.actions {
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

    fn perform(mut self, workdir: PathBuf, current_sha: ObjectId) -> Result<ObjectId, GitOpsError> {
        let deadline = Instant::now() + self.config.timeout;
        let watchers = self.watchers.clone();
        let sink = Arc::new(Mutex::new(move |event: WorkloadEvent| {
            for watcher in &watchers {
                watcher.lock().unwrap()(event.clone())?;
            }
            Ok::<_, GitOpsError>(())
        }));
        let url = self.url_provider.auth_url()?;
        let branch = self.config.git.branch.clone();
        let new_sha = ensure_worktree(url, &branch, deadline, &self.repo_dir, &workdir)?;
        if current_sha != new_sha {
            self.actions.iter_mut().for_each(|action| {
                action.set_env(
                    "KITOPS_LAST_SUCCESSFUL_SHA".to_string(),
                    current_sha.to_string(),
                );
                action.set_env("KITOPS_SHA".to_string(), new_sha.to_string());
            });
            sink.lock().unwrap()(WorkloadEvent::Changes(
                self.config.name.clone(),
                current_sha,
                new_sha,
            ))
            .map_err(|err| GitOpsError::NotifyError(format!("{}", err)))?;
            // TODO The returns dodge cleanup
            match self.run_actions(&workdir, deadline, &sink) {
                Ok(None) => {
                    sink.lock().unwrap()(WorkloadEvent::Success(self.config.name.clone(), new_sha))
                        .map_err(|err| GitOpsError::NotifyError(format!("{}", err)))?
                }
                Ok(Some(action_name)) => {
                    sink.lock().unwrap()(WorkloadEvent::Failure(
                        self.config.name.clone(),
                        action_name.clone(),
                        new_sha,
                    ))
                    .map_err(|err| GitOpsError::NotifyError(format!("{}", err)))?;
                    return Err(GitOpsError::ActionFailed(
                        self.config.name.clone(),
                        action_name,
                    ));
                }
                Err(err) => {
                    sink.lock().unwrap()(WorkloadEvent::Error(
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
