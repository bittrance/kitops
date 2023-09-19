use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread::{scope, sleep},
    time::{Duration, Instant},
};

use gix::{
    bstr::{BString, ByteSlice},
    config::tree::User,
    progress::{AtomicStep, Discard, Id, MessageLevel, Step, StepShared, Unit},
    refs::{
        transaction::{Change, LogChange, RefEdit},
        Target,
    },
    remote::{ref_map::Options, Direction},
    Count, NestedProgress, ObjectId, Progress, Repository, Url,
};
use serde::{Deserialize, Deserializer};

use crate::{errors::GitOpsError, opts::CliOptions};

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

#[derive(Default)]
struct P {
    max: Option<Step>,
    step: Mutex<Option<Step>>,
    unit: Option<Unit>,
    name: Option<String>,
}

impl Progress for P {
    fn init(&mut self, max: Option<Step>, unit: Option<Unit>) {
        self.max = max;
        self.unit = unit;
    }

    fn set_name(&mut self, name: String) {
        println!("created {}", name);
        self.name = Some(name);
    }

    fn name(&self) -> Option<String> {
        self.name.clone()
    }

    fn id(&self) -> Id {
        todo!()
    }

    fn message(&self, level: MessageLevel, message: String) {
        println!("{:?} {:?} {}", self.name, level, message);
    }
}

impl Count for P {
    fn set(&self, step: Step) {
        *self.step.lock().unwrap() = Some(step);
        println!("{:?} start at {:?}", self.name, self.step);
    }

    fn step(&self) -> Step {
        self.step.lock().unwrap().clone().unwrap_or_default()
    }

    fn inc_by(&self, step: Step) {
        *self.step.lock().unwrap().get_or_insert(Step::default()) += step;
        if self.name == Some("read pack".to_owned()) {
            println!("{:?} incremented to {:?}", self.name, self.step);
        }
    }

    fn counter(&self) -> StepShared {
        Arc::new(AtomicUsize::new(
            self.step.lock().unwrap().unwrap_or_default(),
        ))
    }
}

impl NestedProgress for P {
    type SubProgress = P;

    fn add_child(&mut self, name: impl Into<String>) -> Self::SubProgress {
        P {
            name: Some(name.into()),
            ..Default::default()
        }
    }

    fn add_child_with_id(&mut self, name: impl Into<String>, id: Id) -> Self::SubProgress {
        P {
            name: Some(name.into()),
            ..Default::default()
        }
    }
}

fn clone_repo(
    config: &GitConfig,
    deadline: Instant,
    target: &Path,
) -> Result<Repository, GitOpsError> {
    let should_interrupt = AtomicBool::new(false);
    let cancel = AtomicBool::new(false);
    scope(|s| {
        s.spawn(|| {
            while Instant::now() < deadline && !cancel.load(Ordering::Relaxed) {
                sleep(Duration::from_secs(1));
            }
            should_interrupt.store(true, Ordering::Relaxed);
        });
        let p = P::default();
        let (repo, _outcome) = gix::prepare_clone(config.url.clone(), target)
            .unwrap()
            .fetch_only(p, &should_interrupt)
            .map_err(GitOpsError::InitRepo)?;
        cancel.store(true, Ordering::Relaxed);
        Ok(repo)
    })
}

fn fetch_repo(repo: &Repository, config: &GitConfig, deadline: Instant) -> Result<(), GitOpsError> {
    let should_interrupt = AtomicBool::new(false);
    let cancel = AtomicBool::new(false);
    let outcome = scope(|s| {
        s.spawn(|| {
            while Instant::now() < deadline && !cancel.load(Ordering::Relaxed) {
                sleep(Duration::from_secs(1));
            }
            should_interrupt.store(true, Ordering::Relaxed);
        });
        let outcome = repo
            .remote_at(config.url.clone())
            .unwrap()
            .with_refspecs([BString::from(config.branch.clone())], Direction::Fetch)
            .unwrap()
            .connect(Direction::Fetch)
            .map_err(|e| GitOpsError::FetchError(e.into()))?
            .prepare_fetch(Discard, Options::default())
            .map_err(|e| GitOpsError::FetchError(e.into()))?
            .receive(Discard, &should_interrupt)
            .map_err(|e| GitOpsError::FetchError(e.into()))?;
        cancel.store(true, Ordering::Relaxed);
        Ok(outcome)
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
    // let _outcome = gix::worktree::checkout(
    //     &mut state,
    //     workdir,
    //     move |oid, buf| odb.find_blob(oid, buf),
    //     &mut Discard,
    //     &mut Discard,
    //     &AtomicBool::default(),
    //     gix::worktree::checkout::Options::default(),
    // )
    // .unwrap();
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
