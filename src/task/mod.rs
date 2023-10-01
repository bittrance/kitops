use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};

use gix::{hash::Kind, ObjectId, Url};
use serde::{Deserialize, Deserializer, Serialize};

use crate::{actions::Action, errors::GitOpsError, git::GitConfig, opts::CliOptions};

pub mod github;
pub mod gixworkload;
pub mod scheduled;

pub trait Workload {
    fn id(&self) -> String;
    fn interval(&self) -> Duration;
    fn work(&self, workdir: PathBuf, current_sha: ObjectId) -> Result<ObjectId, GitOpsError>;
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

fn human_readable_duration<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    humantime::parse_duration(&s).map_err(serde::de::Error::custom)
}

#[derive(Clone, Deserialize)]
pub struct GitTaskConfig {
    name: String,
    git: GitConfig,
    pub notify: Option<github::GitHubNotifyConfig>,
    actions: Vec<Action>,
    #[serde(
        default = "GitTaskConfig::default_interval",
        deserialize_with = "human_readable_duration"
    )]
    interval: Duration,
    #[serde(
        default = "GitTaskConfig::default_timeout",
        deserialize_with = "human_readable_duration"
    )]
    timeout: Duration,
}

impl GitTaskConfig {
    pub fn add_action(&mut self, action: Action) {
        self.actions.push(action);
    }
}

impl TryFrom<&CliOptions> for GitTaskConfig {
    type Error = GitOpsError;

    fn try_from(opts: &CliOptions) -> Result<Self, Self::Error> {
        let url = Url::try_from(opts.url.clone().unwrap()).map_err(GitOpsError::InvalidUrl)?;
        let action: Action = TryFrom::try_from(opts)?;
        Ok(Self {
            name: url.path.to_string(),
            git: TryFrom::try_from(opts)?,
            notify: TryFrom::try_from(opts)?,
            actions: vec![action],
            interval: opts.interval.unwrap_or(Self::default_interval()),
            timeout: opts.timeout.unwrap_or(Self::default_timeout()),
        })
    }
}

impl GitTaskConfig {
    pub fn default_interval() -> Duration {
        Duration::from_secs(60)
    }

    pub fn default_timeout() -> Duration {
        Duration::from_secs(3600)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::GitTaskConfig;

    #[test]
    fn parse_gittaskconfig() {
        let raw_config = r#"name: testo
git:
  url: https://github.com/bittrance/kitops
timeout: 3s
interval: 1m 2s
actions: []
      "#;
        let config = serde_yaml::from_str::<GitTaskConfig>(raw_config).unwrap();
        assert_eq!(config.timeout, Duration::from_secs(3));
        assert_eq!(config.interval, Duration::from_secs(62));
    }
}
