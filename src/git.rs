use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    thread::{scope, sleep},
    time::{Duration, Instant},
};

use gix::{progress::Discard, ObjectId, Url};
use serde::{Deserialize, Deserializer};

use crate::{errors::GitOpsError, opts::CliOptions};

#[derive(Clone, Deserialize)]
pub struct GitConfig {
    #[serde(deserialize_with = "url_from_string")]
    url: Url,
    // branch: String,
}

impl TryFrom<&CliOptions> for GitConfig {
    type Error = GitOpsError;

    fn try_from(opts: &CliOptions) -> Result<Self, Self::Error> {
        let url = Url::try_from(opts.url.clone().unwrap()).map_err(GitOpsError::InvalidUrl)?;
        Ok(GitConfig { url })
    }
}

fn url_from_string<'de, D>(deserializer: D) -> Result<Url, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    Url::try_from(s).map_err(serde::de::Error::custom)
}

// TODO branch support
pub fn fetch_repo(
    config: GitConfig,
    deadline: Instant,
    target: &Path,
) -> Result<ObjectId, GitOpsError> {
    let should_interrupt = AtomicBool::new(false);
    let cancel = AtomicBool::new(false);
    let repository = scope(|s| {
        s.spawn(|| {
            while Instant::now() < deadline && !cancel.load(Ordering::Relaxed) {
                sleep(Duration::from_secs(1));
            }
            should_interrupt.store(true, Ordering::Relaxed);
        });
        let progress = gix::progress::Discard;
        let (mut checkout, _) = gix::prepare_clone(config.url, target)
            .unwrap()
            .fetch_then_checkout(progress, &should_interrupt)
            .map_err(GitOpsError::InitRepo)?;
        let (repository, _) = checkout
            .main_worktree(Discard, &should_interrupt)
            .map_err(GitOpsError::CheckoutRepo)?;
        cancel.store(true, Ordering::Relaxed);
        Ok(repository)
    })?;
    Ok(repository.head_commit().map(|c| c.id).unwrap())
}
