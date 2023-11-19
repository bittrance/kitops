use std::{
    cell::RefCell,
    path::Path,
    sync::{atomic::AtomicBool, Arc},
    thread::scope,
    time::Instant,
};

use gix::{
    bstr::{BString, ByteSlice},
    config::tree::{
        gitoxide::{self, Credentials},
        Key, User,
    },
    objs::Data,
    odb::{store::Handle, Cache, Store},
    oid,
    progress::Discard,
    refs::{
        transaction::{Change, LogChange, RefEdit},
        Target,
    },
    remote::{fetch::Outcome, ref_map::Options, Direction},
    ObjectId, Repository, Url,
};

use crate::{errors::GitOpsError, utils::Watchdog};

pub trait UrlProvider: Send + Sync {
    fn url(&self) -> &Url;
    fn auth_url(&self) -> Result<Url, GitOpsError>;

    fn safe_url(&self) -> String {
        // TODO Change to whitelist of allowed characters
        self.url().to_bstring().to_string().replace(['/', ':'], "_")
    }
}

#[derive(Clone)]
pub struct DefaultUrlProvider {
    url: Url,
}

impl DefaultUrlProvider {
    pub fn new(url: Url) -> Self {
        DefaultUrlProvider { url }
    }
}

impl UrlProvider for DefaultUrlProvider {
    fn url(&self) -> &Url {
        &self.url
    }

    fn auth_url(&self) -> Result<Url, GitOpsError> {
        Ok(self.url.clone())
    }
}

// TODO What about branch?!
fn clone_repo(url: Url, deadline: Instant, target: &Path) -> Result<Repository, GitOpsError> {
    let watchdog = Watchdog::new(deadline);
    scope(|s| {
        s.spawn(watchdog.runner());
        let maybe_repo = gix::prepare_clone(url, target)
            .unwrap()
            .with_in_memory_config_overrides(vec![gitoxide::Credentials::TERMINAL_PROMPT
                .validated_assignment_fmt(&false)
                .unwrap()])
            .fetch_only(Discard, &watchdog)
            .map(|(r, _)| r)
            .map_err(GitOpsError::InitRepo);
        watchdog.cancel();
        maybe_repo
    })
}

fn perform_fetch(
    repo: &Repository,
    url: Url,
    branch: &str,
    cancel: &AtomicBool,
) -> Result<Outcome, Box<dyn std::error::Error + Send + Sync>> {
    repo.remote_at(url)
        .unwrap()
        .with_refspecs([BString::from(branch)], Direction::Fetch)
        .unwrap()
        .connect(Direction::Fetch)?
        .prepare_fetch(Discard, Options::default())?
        .receive(Discard, cancel)
        .map_err(Into::into)
}

fn fetch_repo(
    repo: &Repository,
    url: Url,
    branch: &str,
    deadline: Instant,
) -> Result<(), GitOpsError> {
    let watchdog = Watchdog::new(deadline);
    let outcome = scope(|s| {
        s.spawn(watchdog.runner());
        let outcome = perform_fetch(repo, url, branch, &watchdog).map_err(GitOpsError::FetchError);
        watchdog.cancel();
        outcome
    })?;
    let needle = BString::from("refs/heads/".to_owned() + branch);
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

#[derive(Clone)]
struct MaybeFind<Allow: Clone, Find: Clone> {
    allow: std::cell::RefCell<Allow>,
    objects: Find,
}

impl<Allow, Find> gix::prelude::Find for MaybeFind<Allow, Find>
where
    Allow: FnMut(&oid) -> bool + Send + Clone,
    Find: gix::prelude::Find + Send + Clone,
{
    fn try_find<'a>(
        &self,
        id: &oid,
        buf: &'a mut Vec<u8>,
    ) -> Result<Option<Data<'a>>, Box<dyn std::error::Error + Send + Sync>> {
        if (self.allow.borrow_mut())(id) {
            self.objects.try_find(id, buf)
        } else {
            Ok(None)
        }
    }
}

fn can_we_please_have_impl_in_type_alias_already() -> impl FnMut(&oid) -> bool + Send + Clone {
    |_| true
}

fn make_finder(odb: Cache<Handle<Arc<Store>>>) -> impl gix::prelude::Find + Send + Clone {
    MaybeFind {
        allow: RefCell::new(can_we_please_have_impl_in_type_alias_already()),
        objects: odb,
    }
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
    let db = make_finder(odb);
    let _outcome = gix::worktree::state::checkout(
        &mut state,
        workdir,
        db,
        &Discard,
        &Discard,
        &AtomicBool::default(),
        gix::worktree::state::checkout::Options::default(),
    )
    .unwrap();
    Ok(oid)
}

pub fn ensure_worktree<P, Q>(
    url: Url,
    branch: &str,
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
        gitconfig
            .set_value(&Credentials::TERMINAL_PROMPT, "false")
            .unwrap();
        gitconfig.commit().unwrap();
        fetch_repo(&repo, url, branch, deadline)?;
        repo
    } else {
        clone_repo(url, deadline, repodir)?
    };
    checkout_worktree(&repo, branch, workdir)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::gix::{clone_repo, fetch_repo};

    const TEST_URL: &str = "https://example.com";

    #[test]
    fn clone_with_bad_url() {
        let deadline = Instant::now() + Duration::from_secs(61); // Fail tests that time out
        let target = tempfile::tempdir().unwrap();
        let result = clone_repo(TEST_URL.try_into().unwrap(), deadline, target.path());
        assert!(result.is_err());
    }

    #[test]
    fn fetch_with_bad_url() {
        let repo = gix::open(".").unwrap();
        let deadline = Instant::now() + Duration::from_secs(61); // Fail tests that time out
        let result = fetch_repo(&repo, TEST_URL.try_into().unwrap(), "main", deadline);
        assert!(result.is_err());
    }
}
