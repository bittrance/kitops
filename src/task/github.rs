use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

use gix::{ObjectId, Url};
use jwt_simple::prelude::{Claims, RS256KeyPair, RSAKeyPairLike};
use reqwest::{
    blocking::ClientBuilder,
    header::{ACCEPT, AUTHORIZATION, USER_AGENT},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{errors::GitOpsError, git::UrlProvider, opts::CliOptions, receiver::WorkloadEvent};

#[derive(Clone, Deserialize)]
pub struct GithubConfig {
    app_id: String,
    private_key_file: PathBuf,
    #[serde(default = "GithubConfig::default_context")]
    pub notify_context: Option<String>,
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
                notify_context: opts.github_context.clone(),
            })),
            _ => Err(GitOpsError::InvalidNotifyConfig),
        }
    }
}

#[derive(Clone)]
pub struct GithubUrlProvider {
    url: Url,
    app_id: String,
    private_key_file: PathBuf,
}

impl GithubUrlProvider {
    pub fn new(url: Url, config: &GithubConfig) -> Self {
        GithubUrlProvider {
            url,
            app_id: config.app_id.clone(),
            private_key_file: config.private_key_file.clone(),
        }
    }

    pub fn repo_slug(&self) -> String {
        self.url.path.to_string().replace(".git", "")[1..].to_owned()
    }
}

impl UrlProvider for GithubUrlProvider {
    fn url(&self) -> &Url {
        &self.url
    }

    fn auth_url(&self) -> Result<Url, GitOpsError> {
        let client = http_client();
        let jwt_token = generate_jwt(&self.app_id, &self.private_key_file)?;
        let installation_id = get_installation_id(&self.repo_slug(), &client, &jwt_token)?;
        let access_token = get_access_token(installation_id, &client, &jwt_token)?;
        // TODO Newer version of gix-url has set_username/set_password
        Ok(Url::from_parts(
            self.url.scheme.clone(),
            Some("x-access-token".to_owned()),
            Some(access_token),
            self.url.host().map(str::to_owned),
            self.url.port,
            self.url.path.clone(),
            false,
        )
        .unwrap())
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

fn http_client() -> reqwest::blocking::Client {
    ClientBuilder::new()
        .connect_timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

fn generate_jwt(app_id: &str, private_key_file: &Path) -> Result<String, GitOpsError> {
    let claims = Claims::create(jwt_simple::prelude::Duration::from_secs(60)).with_issuer(app_id);
    let mut buf = String::with_capacity(1800);
    File::open(private_key_file)
        .map_err(GitOpsError::GitHubMissingPrivateKeyFile)?
        .read_to_string(&mut buf)
        .map_err(GitOpsError::GitHubMissingPrivateKeyFile)?;
    RS256KeyPair::from_pem(&buf)
        .map_err(GitOpsError::GitHubBadPrivateKey)?
        .sign(claims)
        .map_err(GitOpsError::GitHubBadPrivateKey)
}

fn get_installation_id(
    repo_slug: &str,
    client: &reqwest::blocking::Client,
    jwt_token: &String,
) -> Result<u64, GitOpsError> {
    // TODO Is this different if we are installed organization-wise?
    let url = format!("https://api.github.com/repos/{}/installation", repo_slug);
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
    repo_slug: &str,
    config: &GithubConfig,
    sha: &ObjectId,
    status: GitHubStatus,
    message: &str,
) -> Result<(), GitOpsError> {
    let config = config.clone();
    let client = http_client();
    let jwt_token = generate_jwt(&config.app_id, &config.private_key_file)?;
    let installation_id = get_installation_id(repo_slug, &client, &jwt_token)?;
    let access_token = get_access_token(installation_id, &client, &jwt_token)?;

    let url = format!(
        "https://api.github.com/repos/{}/statuses/{}",
        repo_slug, sha
    );
    let body = serde_json::json!({
        "state": status,
        "context": config.notify_context.unwrap(),
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

pub fn github_watcher(
    repo_slug: String,
    config: GithubConfig,
) -> impl Fn(WorkloadEvent) -> Result<(), GitOpsError> + Send + 'static {
    move |event| {
        match event {
            WorkloadEvent::Changes(name, prev_sha, new_sha) => {
                update_commit_status(
                    &repo_slug,
                    &config,
                    &new_sha,
                    GitHubStatus::Pending,
                    &format!("running {} [last success {}]", name, prev_sha),
                )?;
            }
            WorkloadEvent::Success(name, new_sha) => {
                update_commit_status(
                    &repo_slug,
                    &config,
                    &new_sha,
                    GitHubStatus::Success,
                    &format!("{} succeeded", name),
                )?;
            }
            WorkloadEvent::Failure(task, action, new_sha) => {
                update_commit_status(
                    &repo_slug,
                    &config,
                    &new_sha,
                    GitHubStatus::Failure,
                    &format!("{} failed on action {}", task, action),
                )?;
            }
            WorkloadEvent::Error(task, action, new_sha) => {
                update_commit_status(
                    &repo_slug,
                    &config,
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
