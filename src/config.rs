use std::{collections::HashMap, io::Read, path::PathBuf, time::Duration};

use gix::Url;
use serde::{Deserialize, Deserializer};

use crate::{errors::GitOpsError, opts::CliOptions};

#[derive(Deserialize)]
pub struct ConfigFile {
    pub tasks: Vec<GitTaskConfig>,
}

#[derive(Clone, Deserialize)]
pub struct GitTaskConfig {
    pub name: String,
    pub github: Option<GithubConfig>,
    pub git: GitConfig,
    pub actions: Vec<ActionConfig>,
    #[serde(
        default = "GitTaskConfig::default_interval",
        deserialize_with = "human_readable_duration"
    )]
    pub interval: Duration,
    #[serde(
        default = "GitTaskConfig::default_timeout",
        deserialize_with = "human_readable_duration"
    )]
    pub timeout: Duration,
}

impl GitTaskConfig {
    pub fn default_interval() -> Duration {
        Duration::from_secs(60)
    }

    pub fn default_timeout() -> Duration {
        Duration::from_secs(3600)
    }
}

impl TryFrom<&CliOptions> for GitTaskConfig {
    type Error = GitOpsError;

    fn try_from(opts: &CliOptions) -> Result<Self, Self::Error> {
        let url = Url::try_from(opts.url.clone().unwrap()).map_err(GitOpsError::InvalidUrl)?;
        let action: ActionConfig = TryFrom::try_from(opts)?;
        Ok(Self {
            name: url.path.to_string(),
            github: TryFrom::try_from(opts)?,
            git: TryFrom::try_from(opts)?,
            actions: vec![action],
            interval: opts.interval.unwrap_or(Self::default_interval()),
            timeout: opts.timeout.unwrap_or(Self::default_timeout()),
        })
    }
}

#[derive(Clone, Deserialize)]
pub struct GithubConfig {
    pub app_id: String,
    pub private_key_file: PathBuf,
    #[serde(default = "GithubConfig::default_context")]
    pub status_context: Option<String>,
}

impl GithubConfig {
    pub fn default_context() -> Option<String> {
        Some("kitops".to_owned())
    }
}

impl TryFrom<&CliOptions> for Option<GithubConfig> {
    type Error = GitOpsError;

    fn try_from(opts: &CliOptions) -> Result<Self, Self::Error> {
        match (&opts.github_app_id, &opts.github_private_key_file) {
            (None, None) => Ok(None),
            (Some(app_id), Some(private_key_file)) => Ok(Some(GithubConfig {
                app_id: app_id.clone(),
                private_key_file: private_key_file.clone(),
                status_context: opts.github_status_context.clone(),
            })),
            _ => Err(GitOpsError::InvalidNotifyConfig),
        }
    }
}

#[derive(Clone, Deserialize)]
pub struct GitConfig {
    #[serde(deserialize_with = "url_from_string")]
    pub url: Url,
    #[serde(default = "GitConfig::default_branch")]
    pub branch: String,
}

impl GitConfig {
    pub fn default_branch() -> String {
        "main".to_owned()
    }
}

impl TryFrom<&CliOptions> for GitConfig {
    type Error = GitOpsError;

    fn try_from(opts: &CliOptions) -> Result<Self, Self::Error> {
        let url = Url::try_from(opts.url.clone().unwrap()).map_err(GitOpsError::InvalidUrl)?;
        Ok(GitConfig {
            url,
            branch: opts.branch.clone(),
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ActionConfig {
    pub name: String,
    pub entrypoint: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub environment: HashMap<String, String>,
    #[serde(default)]
    pub inherit_environment: bool,
}

impl TryFrom<&CliOptions> for ActionConfig {
    type Error = GitOpsError;

    fn try_from(opts: &CliOptions) -> Result<Self, Self::Error> {
        let mut environment = HashMap::new();
        for env in &opts.environment {
            let (key, val) = env
                .split_once('=')
                .ok_or_else(|| GitOpsError::InvalidEnvVar(env.clone()))?;
            environment.insert(key.to_owned(), val.to_owned());
        }
        Ok(ActionConfig {
            name: opts.action.clone().unwrap(),
            // TODO --action won't work on Windows
            entrypoint: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), opts.action.clone().unwrap()],
            environment,
            inherit_environment: false,
        })
    }
}

fn human_readable_duration<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    humantime::parse_duration(&s).map_err(serde::de::Error::custom)
}

fn url_from_string<'de, D>(deserializer: D) -> Result<Url, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    Url::try_from(s).map_err(serde::de::Error::custom)
}

pub fn read_config(reader: impl Read) -> Result<ConfigFile, GitOpsError> {
    serde_yaml::from_reader(reader).map_err(GitOpsError::MalformedConfig)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::config::GitTaskConfig;

    use super::read_config;

    #[test]
    fn minimum_config() {
        let config = r#"tasks:
  - name: testo
    git:
      url: https://github.com/bittrance/kitops
    actions:
      - name: list files
        entrypoint: /bin/ls
"#;
        read_config(config.as_bytes()).unwrap();
    }

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
