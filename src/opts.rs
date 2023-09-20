use std::{fs::File, path::PathBuf};

use clap::Parser;
use serde::Deserialize;

use crate::{
    actions::Action,
    errors::GitOpsError,
    store::{FileStore, Store},
    task::{GitOpsConfig, GitTask},
};

#[derive(Parser)]
pub struct CliOptions {
    /// Path where state is stored
    #[clap(long, default_value = "./state.yaml")]
    pub state_file: PathBuf,
    /// YAML format task descriptions
    #[clap(long)]
    pub config_file: Option<String>,
    /// Directory to store git repos in
    #[clap(long)]
    pub repo_dir: Option<PathBuf>,
    /// Git repository URL (http(s) for now)
    #[clap(long)]
    pub url: Option<String>,
    /// Branch to check out
    #[clap(long, default_value = "main")]
    pub branch: String,
    /// Command to execute on change (passed to /bin/sh)
    #[clap(long)]
    pub action: Option<String>,
    /// Environment variable for action
    #[clap(long)]
    pub environment: Vec<String>,
    /// Check repo for changes at this interval
    #[clap(long)]
    pub interval: Option<f32>,
    /// Max run time for repo fetch plus action in seconds
    #[clap(long)]
    pub timeout: Option<f32>,
    /// Run once and exit
    #[clap(long)]
    pub once_only: bool,
}

#[derive(Deserialize)]
struct ConfigFile {
    tasks: Vec<GitOpsConfig>,
}

fn tasks_from_file(opts: &CliOptions) -> Result<Vec<GitTask>, GitOpsError> {
    let config =
        File::open(opts.config_file.clone().unwrap()).map_err(GitOpsError::MissingConfig)?;
    let config_file: ConfigFile =
        serde_yaml::from_reader(config).map_err(GitOpsError::MalformedConfig)?;
    Ok(config_file
        .tasks
        .into_iter()
        .map(|c| GitTask::from_config(c, opts))
        .collect())
}

fn tasks_from_opts(opts: &CliOptions) -> Result<Vec<GitTask>, GitOpsError> {
    let mut config: GitOpsConfig = TryFrom::try_from(opts)?;
    let action: Action = TryFrom::try_from(opts)?;
    config.add_action(action);
    Ok(vec![GitTask::from_config(config, opts)])
}

pub fn load_tasks(opts: &CliOptions) -> Result<Vec<GitTask>, GitOpsError> {
    if opts.action.is_some() || opts.url.is_some() {
        if opts.action.is_none() || opts.url.is_none() || opts.config_file.is_some() {
            return Err(GitOpsError::ConfigConflict);
        }
        tasks_from_opts(opts)
    } else {
        if opts.config_file.is_none() {
            return Err(GitOpsError::ConfigConflict);
        }
        tasks_from_file(opts)
    }
}

pub fn load_store(opts: &CliOptions) -> Result<impl Store, GitOpsError> {
    FileStore::from_file(&opts.state_file)
}

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
    serde_yaml::from_str::<ConfigFile>(config).unwrap();
}
