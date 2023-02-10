use clap::Parser;
use git_repository::{hash::Kind, progress::Discard, ObjectId, Url};
use std::{
    collections::HashMap,
    convert::Infallible,
    io::Read,
    ops::Add,
    path::Path,
    process::{Command, ExitStatus, Stdio},
    sync::{
        atomic::AtomicBool,
        mpsc::{channel, Receiver, Sender},
    },
    thread::{sleep, spawn, JoinHandle},
    time::{Duration, Instant},
};
use thiserror::Error;

#[derive(Debug, Error)]
enum GitOpsError {
    #[error("Failed to parse Git repo URL: {0}")]
    InvalidUrl(git_repository::url::parse::Error),
    #[error("Failed to parse environment variable: {0}")]
    InvalidEnvVar(String),
    #[error("Failed to create or locate workdir: {0}")]
    WorkDir(std::io::Error),
    #[error("Failed to create new repository: {0}")]
    InitRepo(git_repository::clone::fetch::Error),
    #[error("Failed to fetch from remote: {0}")]
    CheckoutRepo(git_repository::clone::checkout::main_worktree::Error),
    #[error("Failed to launch action: {0}")]
    ActionError(std::io::Error),
}

impl GitOpsError {
    fn is_fatal(&self) -> bool {
        // TODO Some errors should be recovered
        true
    }
}
struct Task {
    config: GitOpsConfig,
    next_run: Instant,
    current_sha: ObjectId,
    worker: Option<JoinHandle<Result<ObjectId, GitOpsError>>>,
}

#[derive(Clone)]
struct GitOpsConfig {
    git: GitConfig,
    actions: Vec<Action>,
}

#[derive(Clone)]
struct GitConfig {
    url: Url,
    // branch: String,
}

#[derive(Clone)]
struct Action {
    name: String,
    entrypoint: String,
    args: Vec<String>,
    environment: HashMap<String, String>,
    timeout: Duration,
}

enum ActionOutput {
    Stdout(String, Vec<u8>),
    Stderr(String, Vec<u8>),
    Exit(String, ExitStatus),
    Timeout(String),
}

fn build_command(action: &Action, cwd: &Path) -> Command {
    let mut command = Command::new(action.entrypoint.clone());
    command.args(action.args.clone());
    command.envs(action.environment.iter());
    command.current_dir(cwd);
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command
}

fn run_action(
    name: &str,
    action: &Action,
    cwd: &Path,
    tx: &Sender<ActionOutput>,
) -> Result<(), GitOpsError> {
    let mut command = build_command(action, cwd);
    let mut child = command.spawn().map_err(GitOpsError::ActionError)?;
    // TODO Coordinate deadline with fetch
    let deadline = Instant::now() + action.timeout;
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let mut buf: [u8; 4096] = [0; 4096];
    let as_action_error =
        |err| GitOpsError::ActionError(std::io::Error::new(std::io::ErrorKind::Other, err));
    loop {
        if stderr.read(&mut buf).map_err(GitOpsError::ActionError)? > 0 {
            tx.send(ActionOutput::Stderr(name.to_string(), buf.into()))
                .map_err(as_action_error)?;
            continue;
        }
        if stdout.read(&mut buf).map_err(GitOpsError::ActionError)? > 0 {
            tx.send(ActionOutput::Stdout(name.to_string(), buf.into()))
                .map_err(as_action_error)?;
            continue;
        }
        if let Some(exit) = child.try_wait().map_err(GitOpsError::ActionError)? {
            tx.send(ActionOutput::Exit(name.to_string(), exit))
                .map_err(as_action_error)?;
            break;
        }
        if Instant::now() > deadline {
            child.kill().map_err(GitOpsError::ActionError)?;
            tx.send(ActionOutput::Timeout(name.to_string()))
                .map_err(as_action_error)?;
            break;
        }
        sleep(Duration::from_secs(1));
    }
    Ok(())
}

// TODO SSH support
// TODO branch support
fn fetch_repo(config: GitConfig, target: &Path) -> Result<ObjectId, GitOpsError> {
    let should_interrupt = AtomicBool::new(false);
    let progress = git_repository::progress::Discard;
    let (mut checkout, _) = git_repository::prepare_clone(config.url, target)
        .unwrap()
        .fetch_then_checkout(progress, &should_interrupt)
        .map_err(GitOpsError::InitRepo)?;
    let (repository, _) = checkout
        .main_worktree(Discard, &should_interrupt)
        .map_err(GitOpsError::CheckoutRepo)?;
    Ok(repository.head_commit().map(|c| c.id).unwrap())
}

fn eligible_task(task: &Task) -> bool {
    task.worker.is_none() && task.next_run < Instant::now()
}

fn finished_task(task: &Task) -> bool {
    task.worker.as_ref().map(JoinHandle::is_finished).is_some()
}

fn run(tasks: &mut [Task], tx: &Sender<ActionOutput>) -> Result<Infallible, GitOpsError> {
    loop {
        if let Some(mut task) = tasks.iter_mut().find(|t| eligible_task(t)) {
            let config = task.config.clone();
            let reponame = config.git.url.path.to_string();
            let current_sha = task.current_sha;
            let workdir = tempfile::tempdir()
                .map_err(GitOpsError::WorkDir)?
                .into_path();
            let task_tx = tx.clone();
            let worker = spawn(move || {
                let new_sha = fetch_repo(config.git, &workdir)?;
                if current_sha != new_sha {
                    for action in config.actions {
                        let name = format!("{}|{}", reponame, action.name);
                        run_action(&name, &action, &workdir, &task_tx)?;
                    }
                }
                std::fs::remove_dir_all(workdir).map_err(GitOpsError::WorkDir)?;
                Ok(new_sha)
            });
            task.worker = Some(worker);
            task.next_run = Instant::now().add(Duration::from_secs(60));
            continue;
        }
        if let Some(mut task) = tasks.iter_mut().find(|t| finished_task(t)) {
            let worker = task.worker.take().unwrap();
            match worker.join().unwrap() {
                Ok(new_sha) => task.current_sha = new_sha,
                Err(err) if err.is_fatal() => return Err(err),
                Err(_) => (),
            }
            continue;
        }
        sleep(Duration::from_secs(1));
    }
}

fn logging_receiver(events: &Receiver<ActionOutput>) {
    while let Ok(event) = events.recv() {
        match event {
            ActionOutput::Stdout(name, data) => {
                print!("{}: {}", name, String::from_utf8_lossy(&data));
            }
            ActionOutput::Stderr(name, data) => {
                eprint!("{}: {}", name, String::from_utf8_lossy(&data));
            }
            ActionOutput::Exit(name, exit) => println!("{}: exited with code {}", name, exit),
            ActionOutput::Timeout(name) => println!("{}: took too long", name),
        }
    }
}

#[derive(Parser)]
struct CliOptions {
    /// Git repository URL (http(s) for now)
    #[clap(long)]
    url: String,
    // /// Branch to check out
    // #[clap(long)]
    // branch: String,
    /// Command to execute on change (passed to /bin/sh)
    #[clap(long)]
    action: String,
    /// Environment variable for action
    #[clap(long)]
    environment: Vec<String>,
    /// Max run time for repo fetch plus action in seconds
    #[clap(long)]
    timeout: f32,
}

fn task_from_opts(opts: &CliOptions) -> Result<Task, GitOpsError> {
    let url = Url::try_from(opts.url.clone()).map_err(GitOpsError::InvalidUrl)?;
    let mut environment = HashMap::new();
    for env in &opts.environment {
        let (key, val) = env
            .split_once('=')
            .ok_or_else(|| GitOpsError::InvalidEnvVar(env.clone()))?;
        environment.insert(key.to_owned(), val.to_owned());
    }
    let action = Action {
        name: opts.action.clone(),
        entrypoint: opts.action.clone(),
        args: vec![],
        environment,
        timeout: Duration::from_secs_f32(opts.timeout),
    };
    let actions = vec![action];
    Ok(Task {
        config: GitOpsConfig {
            git: GitConfig {
                url, /* branch: opts.branch.clone() */
            },
            actions,
        },
        current_sha: ObjectId::null(Kind::Sha1),
        next_run: Instant::now(),
        worker: None,
    })
}

fn main() -> Result<Infallible, GitOpsError> {
    let opts = CliOptions::parse();
    let task = task_from_opts(&opts)?;
    // TODO deserialize tasks from file
    let (tx, rx) = channel();
    // TODO Handle TERM both here and when running actions
    spawn(move || {
        logging_receiver(&rx);
    });
    run(&mut [task], &tx)
}
