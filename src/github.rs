use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

use gix::{url::Scheme, ObjectId, Url};
use jwt_simple::prelude::{Claims, RS256KeyPair, RSAKeyPairLike};
use reqwest::{
    blocking::ClientBuilder,
    header::{ACCEPT, AUTHORIZATION, USER_AGENT},
};
use serde::Serialize;
use serde_json::Value;

use crate::{config::GithubConfig, errors::GitOpsError, gix::UrlProvider, receiver::WorkloadEvent};

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
        if self.url.scheme != Scheme::Https {
            let mut buf = Vec::new();
            self.url.write_to(&mut buf).unwrap();
            let url_str = String::from_utf8(buf).unwrap_or_else(|_| "<unparseable>".to_owned());
            return Err(GitOpsError::GitHubAuthNonHttpsUrl(url_str));
        }
        let client = http_client();
        let jwt_token = generate_jwt(&self.app_id, &self.private_key_file)?;
        let installation_id = get_installation_id(&self.repo_slug(), &client, &jwt_token)?;
        let access_token = get_access_token(installation_id, &client, &jwt_token)?;
        let mut auth_url = self.url.clone();
        auth_url.set_user(Some("x-access-token".to_owned()));
        auth_url.set_password(Some(access_token));
        Ok(auth_url)
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
        "context": config.status_context,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn github_url_provider_slug() {
        let url = Url::try_from("https://github.com/bittrance/kitops.git".to_owned()).unwrap();
        let config = GithubConfig {
            app_id: "1234".to_owned(),
            private_key_file: PathBuf::from("ze-key"),
            status_context: Some("ze-context".to_owned()),
        };
        let provider = GithubUrlProvider::new(url, &config);
        assert_eq!(provider.repo_slug(), "bittrance/kitops");
    }

    #[test]
    fn github_url_provider_refuses_http_on_auth() {
        let url = Url::try_from("http://some.site/bittrance/kitops".to_owned()).unwrap();
        let config = GithubConfig {
            app_id: "1234".to_owned(),
            private_key_file: PathBuf::from("ze-key"),
            status_context: Some("ze-context".to_owned()),
        };
        let provider = GithubUrlProvider::new(url, &config);
        assert!(matches!(
            provider.auth_url(),
            Err(GitOpsError::GitHubAuthNonHttpsUrl(_))
        ));
    }
}
