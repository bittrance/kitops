use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};

use gix::{hash::Kind, ObjectId};
use serde::{Deserialize, Serialize};

use crate::errors::GitOpsError;

pub mod github;
pub mod gitworkload;
pub mod scheduled;

pub trait Workload {
    fn id(&self) -> String;
    fn interval(&self) -> Duration;
    fn perform(self, workdir: PathBuf, current_sha: ObjectId) -> Result<ObjectId, GitOpsError>;
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
