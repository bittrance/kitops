use std::{path::Path, sync::atomic::AtomicBool, thread::scope, time::Instant};

use gix::{
    bstr::{BString, ByteSlice},
    config::tree::User,
    prelude::FindExt,
    progress::Discard,
    refs::{
        transaction::{Change, LogChange, RefEdit},
        Target,
    },
    remote::{fetch::Outcome, ref_map::Options, Direction},
    ObjectId, Repository, Url,
};
use serde::{Deserialize, Deserializer};

use crate::{errors::GitOpsError, opts::CliOptions, utils::Watchdog};

#[derive(Clone, Deserialize)]
pub struct GitConfig {
    #[serde(deserialize_with = "url_from_string")]
    url: Url,
    #[serde(default = "GitConfig::default_branch")]
    branch: String,
}

impl GitConfig {
    pub fn safe_url(&self) -> String {
        // TODO Change to whitelist of allowed characters
        self.url.to_bstring().to_string().replace(['/', ':'], "_")
    }

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

fn url_from_string<'de, D>(deserializer: D) -> Result<Url, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    Url::try_from(s).map_err(serde::de::Error::custom)
}

fn clone_repo(
    config: &GitConfig,
    deadline: Instant,
    target: &Path,
) -> Result<Repository, GitOpsError> {
    let watchdog = Watchdog::new(deadline);
    scope(|s| {
        s.spawn(watchdog.runner());
        let repo = gix::prepare_clone(config.url.clone(), target)
            .unwrap()
            .fetch_only(Discard, &watchdog)
            .map(|(r, _)| r)
            .map_err(GitOpsError::InitRepo);
        watchdog.cancel();
        repo
    })
}

fn perform_fetch(
    repo: &Repository,
    config: &GitConfig,
    cancel: &AtomicBool,
) -> Result<Outcome, Box<dyn std::error::Error + Send + Sync>> {
    repo.remote_at(config.url.clone())
        .unwrap()
        .with_refspecs([BString::from(config.branch.clone())], Direction::Fetch)
        .unwrap()
        .connect(Direction::Fetch)?
        .prepare_fetch(Discard, Options::default())?
        .receive(Discard, cancel)
        .map_err(Into::into)
}

fn fetch_repo(repo: &Repository, config: &GitConfig, deadline: Instant) -> Result<(), GitOpsError> {
    let watchdog = Watchdog::new(deadline);
    let outcome = scope(|s| {
        s.spawn(watchdog.runner());
        let outcome = perform_fetch(repo, config, &watchdog).map_err(GitOpsError::FetchError);
        watchdog.cancel();
        outcome
    })?;
    let needle = BString::from("refs/heads/".to_owned() + &config.branch);
    let target = outcome
        .ref_map
        .remote_refs
        .iter()
        .map(|r| r.unpack())
        .find_map(|(name, oid, _)| if name == needle.as_bstr() { oid } else { None })
        .unwrap()
        .to_owned();
    let edit = RefEdit {
        change: Change::Update {
            log: LogChange {
                mode: gix::refs::transaction::RefLog::AndReference,
                force_create_reflog: false,
                message: BString::from("kitops fetch"),
            },
            expected: gix::refs::transaction::PreviousValue::Any,
            new: Target::Peeled(target),
        },
        name: needle.try_into().unwrap(),
        deref: false,
    };
    repo.edit_reference(edit).unwrap();
    Ok(())
}

fn checkout_worktree(
    repo: &Repository,
    branch: &str,
    workdir: &Path,
) -> Result<ObjectId, GitOpsError> {
    let oid = repo
        .refs
        .find(branch)
        .unwrap()
        .target
        .try_into_id()
        .unwrap();
    let tree_id = repo
        .find_object(oid)
        .unwrap()
        .into_commit()
        .tree_id()
        .unwrap();
    let (mut state, _) = repo.index_from_tree(&tree_id).unwrap().into_parts();
    let odb = repo.objects.clone().into_arc().unwrap();
    let _outcome = gix::worktree::state::checkout(
        &mut state,
        workdir,
        move |oid, buf| odb.find_blob(oid, buf),
        &Discard,
        &Discard,
        &AtomicBool::default(),
        gix::worktree::state::checkout::Options::default(),
    )
    .unwrap();
    Ok(oid)
}

pub fn ensure_worktree<P, Q>(
    config: &GitConfig,
    deadline: Instant,
    repodir: P,
    workdir: Q,
) -> Result<ObjectId, GitOpsError>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    let repodir = repodir.as_ref();
    let workdir = workdir.as_ref();
    let repo = if repodir.join(".git").try_exists().unwrap() {
        let mut repo = gix::open(repodir).map_err(GitOpsError::OpenRepo)?;
        // TODO Workaround for gitoxide not supporting empty user.email
        let mut gitconfig = repo.config_snapshot_mut();
        gitconfig.set_value(&User::NAME, "kitops").unwrap();
        gitconfig.set_value(&User::EMAIL, "none").unwrap();
        gitconfig.commit().unwrap();
        fetch_repo(&repo, config, deadline)?;
        repo
    } else {
        clone_repo(config, deadline, repodir)?
    };
    checkout_worktree(&repo, &config.branch, workdir)
}
