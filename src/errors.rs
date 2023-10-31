use std::{path::PathBuf, fmt::Debug};

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
    #[error("Notify section needs github_repo_slug and github_context")]
    InvalidNotifyConfig,
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
    #[error("Action failed: {1} in {0}")]
    ActionFailed(String, String),
    #[error("Failed to send event: {0}")]
    NotifyError(String),
    #[error("Failed to launch action: {0}")]
    ActionError(std::io::Error),
    #[error("Missing private key file: {0}")]
    GitHubMissingPrivateKeyFile(std::io::Error),
    #[error("Malformed private RS256 key: {0}")]
    GitHubBadPrivateKey(jwt_simple::Error),
    #[error("GitHub API {0} returned status {1}: {2}")]
    GitHubApiError(String, reqwest::StatusCode, String),
    #[error("Failed to connect to GitHub API: {0}")]
    GitHubNetworkError(reqwest::Error),
    #[error("GitHub App is installed but does not have write permissions for commit statuses")]
    GitHubPermissionsError,
}

impl GitOpsError {
    #[allow(clippy::unused_self)]
    pub fn is_fatal(&self) -> bool {
        #[allow(clippy::match_like_matches_macro)]
        match self {
            Self::ActionFailed(..) => false,
            _ => true,
        }
    }
}
