use std::{fs::File, io::Read, path::PathBuf, time::Duration};

use gix::ObjectId;
use jwt_simple::prelude::{Claims, RS256KeyPair, RSAKeyPairLike};
use reqwest::{
    blocking::ClientBuilder,
    header::{ACCEPT, AUTHORIZATION, USER_AGENT},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{errors::GitOpsError, opts::CliOptions, receiver::ActionOutput};

#[derive(Clone, Deserialize)]
pub struct GitHubNotifyConfig {
    app_id: String,
    private_key_file: PathBuf,
    repo_slug: String,
    #[serde(default = "GitHubNotifyConfig::default_context")]
    context: String,
}

impl GitHubNotifyConfig {
    pub fn default_context() -> String {
        "kitops".to_owned()
    }
}

impl TryFrom<&CliOptions> for Option<GitHubNotifyConfig> {
    type Error = GitOpsError;

    fn try_from(opts: &CliOptions) -> Result<Self, Self::Error> {
        match (
            &opts.github_app_id,
            &opts.github_private_key_file,
            &opts.github_repo_slug,
            &opts.github_context,
        ) {
            (None, None, None, None) => Ok(None),
            (Some(app_id), Some(private_key_file), Some(repo_slug), Some(context)) => {
                Ok(Some(GitHubNotifyConfig {
                    app_id: app_id.clone(),
                    private_key_file: private_key_file.clone(),
                    repo_slug: repo_slug.clone(),
                    context: context.clone(),
                }))
            }
            _ => Err(GitOpsError::InvalidNotifyConfig),
        }
    }
}

#[derive(Serialize)]
pub enum GitHubStatus {
    #[serde(rename = "pending")]
    Pending,
    #[serde(rename = "success")]
    Success,
    #[serde(rename = "failure")]
    Failure,
    #[serde(rename = "error")]
    Error,
}

fn generate_jwt(config: &GitHubNotifyConfig) -> Result<String, GitOpsError> {
    let claims = Claims::create(jwt_simple::prelude::Duration::from_secs(60))
        .with_issuer(config.app_id.clone());
    let mut buf = String::with_capacity(1800);
    File::open(&config.private_key_file)
        .map_err(GitOpsError::GitHubMissingPrivateKeyFile)?
        .read_to_string(&mut buf)
        .map_err(GitOpsError::GitHubMissingPrivateKeyFile)?;
    RS256KeyPair::from_pem(&buf)
        .map_err(GitOpsError::GitHubBadPrivateKey)?
        .sign(claims)
        .map_err(GitOpsError::GitHubBadPrivateKey)
}

fn get_installation_id(
    config: &GitHubNotifyConfig,
    client: &reqwest::blocking::Client,
    jwt_token: &String,
) -> Result<u64, GitOpsError> {
    // TODO Is this different if we are installed organization-wise?
    let url = format!(
        "https://api.github.com/repos/{}/installation",
        config.repo_slug
    );
    let res = client
        .get(&url)
        .header(ACCEPT, "application/vnd.github+json")
        .header(AUTHORIZATION, format!("Bearer {}", jwt_token))
        .header(USER_AGENT, "bittrance/kitops")
        .send()
        .map_err(GitOpsError::GitHubNetworkError)?;
    if !res.status().is_success() {
        return Err(GitOpsError::GitHubApiError(
            url,
            res.status(),
            res.text()
                .unwrap_or("GitHub Api returned unparseable error".to_owned()),
        ));
    }
    let installation: Value = res.json().unwrap();
    let installation_id = installation["id"].as_u64().unwrap();
    let permissions = installation["permissions"].as_object().unwrap();
    if permissions.get("statuses") != Some(&Value::String("write".to_owned())) {
        return Err(GitOpsError::GitHubPermissionsError);
    }
    Ok(installation_id)
}

fn get_access_token(
    installation_id: u64,
    client: &reqwest::blocking::Client,
    jwt_token: &String,
) -> Result<String, GitOpsError> {
    let url = format!(
        "https://api.github.com/app/installations/{}/access_tokens",
        installation_id
    );
    let res = client
        .post(&url)
        .header(ACCEPT, "application/vnd.github+json")
        .header(AUTHORIZATION, format!("Bearer {}", jwt_token))
        .header(USER_AGENT, "bittrance/kitops")
        .send()
        .map_err(GitOpsError::GitHubNetworkError)?;
    if !res.status().is_success() {
        return Err(GitOpsError::GitHubApiError(
            url,
            res.status(),
            res.text()
                .unwrap_or("GitHub Api returned unparseable error".to_owned()),
        ));
    }
    let access: Value = res.json().unwrap();
    let access_token = access["token"].as_str().unwrap().to_owned();
    Ok(access_token)
}

pub fn update_commit_status(
    config: &GitHubNotifyConfig,
    sha: &ObjectId,
    status: GitHubStatus,
    message: &str,
) -> Result<(), GitOpsError> {
    let client = ClientBuilder::new()
        .connect_timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    let jwt_token = generate_jwt(config)?;
    let installation_id = get_installation_id(config, &client, &jwt_token)?;
    let access_token = get_access_token(installation_id, &client, &jwt_token)?;

    let url = format!(
        "https://api.github.com/repos/{}/statuses/{}",
        config.repo_slug, sha
    );
    let body = serde_json::json!({
        "state": status,
        "context": config.context,
        "description": message,
    });
    let res = client
        .post(&url)
        .header(AUTHORIZATION, format!("Bearer {}", access_token))
        .header(USER_AGENT, "bittrance/kitops")
        .json(&body)
        .send()
        .map_err(GitOpsError::GitHubNetworkError)?;
    if res.status().is_success() {
        Ok(())
    } else {
        Err(GitOpsError::GitHubApiError(
            url,
            res.status(),
            res.text()
                .unwrap_or("GitHub Api returned unparseable error".to_owned()),
        ))
    }
}

pub fn github_watcher(notify_config: GitHubNotifyConfig) -> impl Fn(ActionOutput) -> Result<(), GitOpsError> + Send + 'static {
    move |event| {
        match event {
            ActionOutput::Changes(name, prev_sha, new_sha) => {
                update_commit_status(
                    &notify_config,
                    &new_sha,
                    GitHubStatus::Pending,
                    &format!("running {} [last success {}]", name, prev_sha),
                )?;
            }
            ActionOutput::Success(name, new_sha) => {
                update_commit_status(
                    &notify_config,
                    &new_sha,
                    GitHubStatus::Success,
                    &format!("{} succeeded", name),
                )?;
            }
            ActionOutput::Failure(task, action, new_sha) => {
                update_commit_status(
                    &notify_config,
                    &new_sha,
                    GitHubStatus::Failure,
                    &format!("{} failed on action {}", task, action),
                )?;
            }
            ActionOutput::Error(task, action, new_sha) => {
                update_commit_status(
                    &notify_config,
                    &new_sha,
                    GitHubStatus::Error,
                    &format!("{} errored on action {}", task, action),
                )?;
            }
            _ => (),
        };
        Ok(())
    }
}