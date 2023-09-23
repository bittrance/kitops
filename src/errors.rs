use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitOpsError {
    #[error("Failed to parse Git repo URL: {0}")]
    InvalidUrl(gix::url::parse::Error),
    #[error("Failed to parse environment variable: {0}")]
    InvalidEnvVar(String),
    #[error("Config file not found: {0}")]
    MissingConfig(std::io::Error),
    #[error("Malformed configuration: {0}")]
    MalformedConfig(serde_yaml::Error),
    #[error("Provide --url and --action or --config-file")]
    ConfigMethodConflict,
    #[error("Provide --interval or --once-only")]
    ConfigExecutionConflict,
    #[error("Cannot find directory to store repositories: {0}")]
    MissingRepoDir(PathBuf),
    #[error("Failed to create directory to store repositories: {0}")]
    CreateRepoDir(std::io::Error),
    #[error("Failed to open/create state file: {0}")]
    StateFile(std::io::Error),
    #[error("Falied to read state: {0}")]
    LoadingState(std::io::Error),
    #[error("Failed to write state: {0}")]
    SavingState(std::io::Error),
    #[error("Failed to de/serialize state: {0}")]
    SerdeState(serde_yaml::Error),
    #[error("Failed to create or locate workdir: {0}")]
    WorkDir(std::io::Error),
    #[error("Failed to create new repository: {0}")]
    InitRepo(gix::clone::fetch::Error),
    #[error("Failed to connect to remote: {0}")]
    FetchError(Box<dyn std::error::Error + Send + Sync>),
    #[error("Failed to open repository: {0}")]
    OpenRepo(gix::open::Error),
    #[error("Failed to send event: {0}")]
    SendError(String),
    #[error("Failed to launch action: {0}")]
    ActionError(std::io::Error),
}

impl GitOpsError {
    #[allow(clippy::unused_self)]
    pub fn is_fatal(&self) -> bool {
        // TODO Some errors should be recovered
        true
    }
}
