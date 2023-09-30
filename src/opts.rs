use std::{fs::File, path::PathBuf, sync::mpsc::channel, thread::spawn, time::Duration};

use clap::Parser;
use serde::Deserialize;

use crate::{
    actions::Action,
    errors::GitOpsError,
    receiver::{logging_receiver, ActionOutput},
    store::{FileStore, Store},
    task::{
        github::{update_commit_status, GitHubStatus},
        gixworkload::GitWorkload,
        scheduled::ScheduledTask,
        GitTaskConfig,
    },
};

const DEFAULT_BRANCH: &str = "main";

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
    #[clap(long, default_value = DEFAULT_BRANCH)]
    pub branch: String,
    /// Command to execute on change (passed to /bin/sh)
    #[clap(long)]
    pub action: Option<String>,
    /// Environment variable for action
    #[clap(long)]
    pub environment: Vec<String>,
    /// GitHub App ID
    #[clap(long)]
    pub github_app_id: Option<String>,
    /// GitHub App private key file
    #[clap(long)]
    pub github_private_key_file: Option<PathBuf>,
    /// Update GitHub commit status on this repo
    #[clap(long)]
    pub github_repo_slug: Option<String>,
    /// Use this context when updating GitHub commit status
    #[clap(long)]
    pub github_context: Option<String>,
    /// Check repo for changes at this interval (e.g. 1h, 30m, 10s)
    #[arg(long, value_parser = humantime::parse_duration)]
    pub interval: Option<Duration>,
    /// Max run time for repo fetch plus action (e.g. 1h, 30m, 10s)
    #[arg(long, value_parser = humantime::parse_duration)]
    pub timeout: Option<Duration>,
    /// Run once and exit
    #[clap(long)]
    pub once_only: bool,
}

impl CliOptions {
    pub fn complete(&mut self) -> Result<(), GitOpsError> {
        if self.config_file.is_some() {
            if self.url.is_some()
                || self.branch != DEFAULT_BRANCH
                || self.action.is_some()
                || !self.environment.is_empty()
            {
                return Err(GitOpsError::ConfigMethodConflict);
            }
        } else if self.url.is_none() || self.action.is_none() {
            return Err(GitOpsError::ConfigMethodConflict);
        }
        if self.once_only && self.interval.is_some() {
            return Err(GitOpsError::ConfigExecutionConflict);
        }
        if let Some(ref dir) = self.repo_dir {
            if !dir.exists() {
                return Err(GitOpsError::MissingRepoDir(dir.clone()));
            }
        } else {
            self.repo_dir = Some(
                tempfile::tempdir()
                    .map_err(GitOpsError::CreateRepoDir)?
                    .into_path(),
            );
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct ConfigFile {
    tasks: Vec<GitTaskConfig>,
}

fn into_task(mut config: GitTaskConfig, opts: &CliOptions) -> ScheduledTask<GitWorkload> {
    let notify_config = config.notify.take();
    let mut work = GitWorkload::from_config(config, opts);
    if let Some(notify_config) = notify_config {
        work.watch(move |event| {
            match event {
                // TODO Need to wire for failure/error
                ActionOutput::Success(_, new_sha) => {
                    update_commit_status(
                        &notify_config,
                        &new_sha,
                        GitHubStatus::Success,
                        "success",
                    )?;
                }
                _ => (),
            };
            Ok(())
        });
    }
    let (tx, rx) = channel();
    work.watch(move |event| {
        tx.send(event)
            .map_err(|e| GitOpsError::NotifyError(format!("{}", e)))
    });
    // TODO Handle TERM
    spawn(move || {
        logging_receiver(&rx);
    });
    ScheduledTask::new(work)
}

fn tasks_from_file(opts: &CliOptions) -> Result<Vec<ScheduledTask<GitWorkload>>, GitOpsError> {
    let config =
        File::open(opts.config_file.clone().unwrap()).map_err(GitOpsError::MissingConfig)?;
    let config_file: ConfigFile =
        serde_yaml::from_reader(config).map_err(GitOpsError::MalformedConfig)?;
    Ok(config_file
        .tasks
        .into_iter()
        .map(|c| into_task(c, opts))
        .collect())
}

fn tasks_from_opts(opts: &CliOptions) -> Result<Vec<ScheduledTask<GitWorkload>>, GitOpsError> {
    let mut config: GitTaskConfig = TryFrom::try_from(opts)?;
    let action: Action = TryFrom::try_from(opts)?;
    config.add_action(action);
    Ok(vec![into_task(config, opts)])
}

pub fn load_tasks(opts: &CliOptions) -> Result<Vec<ScheduledTask<GitWorkload>>, GitOpsError> {
    if opts.url.is_some() {
        tasks_from_opts(opts)
    } else {
        tasks_from_file(opts)
    }
}

pub fn load_store(opts: &CliOptions) -> Result<impl Store, GitOpsError> {
    FileStore::from_file(&opts.state_file)
}

#[test]
fn complete_cli_options_no_args() {
    let mut opts = CliOptions::parse_from(&["kitops"]);
    let res = opts.complete();
    assert!(matches!(res, Err(GitOpsError::ConfigMethodConflict)));
}

#[test]
fn complete_cli_options_incomplete_args() {
    let mut opts = CliOptions::parse_from(&["kitops", "--url", "file:///tmp"]);
    let res = opts.complete();
    assert!(matches!(res, Err(GitOpsError::ConfigMethodConflict)));
}

#[test]
fn complete_cli_options_conflicting_args() {
    let mut opts = CliOptions::parse_from(&[
        "kitops",
        "--config-file",
        "foo.yaml",
        "--url",
        "file:///tmp",
    ]);
    let res = opts.complete();
    assert!(matches!(res, Err(GitOpsError::ConfigMethodConflict)));
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
