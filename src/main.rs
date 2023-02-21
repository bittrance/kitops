use clap::Parser;
use gix::{hash::Kind, progress::Discard, ObjectId, Url};
use serde::{Deserialize, Deserializer, Serialize};
use std::{
    collections::{HashMap, HashSet},
    convert::Infallible,
    fs::File,
    io::Read,
    ops::Add,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{channel, Receiver, SendError, Sender},
    },
    thread::{scope, sleep, spawn, JoinHandle},
    time::{Duration, Instant, SystemTime},
};
use thiserror::Error;

#[derive(Debug, Error)]
enum GitOpsError {
    #[error("Failed to parse Git repo URL: {0}")]
    InvalidUrl(gix::url::parse::Error),
    #[error("Failed to parse environment variable: {0}")]
    InvalidEnvVar(String),
    #[error("Config file not found: {0}")]
    MissingConfig(std::io::Error),
    #[error("Malformed configuration: {0}")]
    MalformedConfig(serde_yaml::Error),
    #[error("Provide --url and --action or --config-file")]
    ConfigConflict,
    #[error("Failed to open/create state file: {0}")]
    StateFile(std::io::Error),
    #[error("Falied to read state: {0}")]
    LoadingState(std::io::Error),
    #[error("Failed to write state: {0}")]
    SavingState(std::io::Error),
    #[error("Failed to de/serialize state: {0}")]
    SerdeState(serde_yaml::Error),
    #[error("Failed to create or locate workdir: {0}")]
    WorkDir(std::io::Error),
    #[error("Failed to create new repository: {0}")]
    InitRepo(gix::clone::fetch::Error),
    #[error("Failed to fetch from remote: {0}")]
    CheckoutRepo(gix::clone::checkout::main_worktree::Error),
    #[error("Failed to send event: {0}")]
    SendError(SendError<ActionOutput>),
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
    state: State,
    worker: Option<JoinHandle<Result<ObjectId, GitOpsError>>>,
}

impl Task {
    fn id(&self) -> String {
        format!("{}", self.config.git.url.to_bstring())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct State {
    next_run: SystemTime,
    current_sha: ObjectId,
}

impl Default for State {
    fn default() -> Self {
        Self {
            current_sha: ObjectId::null(Kind::Sha1),
            next_run: SystemTime::now(),
        }
    }
}

#[derive(Deserialize)]
struct ConfigFile {
    tasks: Vec<GitOpsConfig>,
}

#[derive(Clone, Deserialize)]
struct GitOpsConfig {
    name: String,
    git: GitConfig,
    actions: Vec<Action>,
    #[serde(default = "default_interval")]
    interval: Duration,
    #[serde(default = "default_timeout")]
    timeout: Duration,
}

fn url_from_string<'de, D>(deserializer: D) -> Result<Url, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    Url::try_from(s).map_err(serde::de::Error::custom)
}

fn default_interval() -> Duration {
    Duration::from_secs(60)
}

fn default_timeout() -> Duration {
    Duration::from_secs(3600)
}

#[derive(Clone, Deserialize)]
struct GitConfig {
    #[serde(deserialize_with = "url_from_string")]
    url: Url,
    // branch: String,
}

#[derive(Clone, Deserialize)]
struct Action {
    name: String,
    entrypoint: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    environment: HashMap<String, String>,
    #[serde(default)]
    inherit_environment: bool,
}

#[derive(Clone, Copy)]
enum SourceType {
    StdOut,
    StdErr,
}

enum ActionOutput {
    Changes(String, ObjectId, ObjectId),
    Output(String, SourceType, Vec<u8>),
    Exit(String, ExitStatus),
    Timeout(String),
}

fn build_command(action: &Action, cwd: &Path) -> Command {
    let mut command = Command::new(action.entrypoint.clone());
    command.args(action.args.clone());
    if !action.inherit_environment {
        command.env_clear();
        if let Ok(path) = std::env::var("PATH") {
            command.env("PATH", path);
        }
    }
    command.envs(action.environment.iter());
    command.current_dir(cwd);
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command
}

fn emit_data<R>(
    name: String,
    mut source: R,
    source_type: SourceType,
    tx: Sender<ActionOutput>,
) -> JoinHandle<Result<(), GitOpsError>>
where
    R: Read + Send + 'static,
{
    spawn(move || {
        let mut buf: [u8; 4096] = [0; 4096];
        while source.read(&mut buf).map_err(GitOpsError::ActionError)? > 0 {
            tx.send(ActionOutput::Output(name.clone(), source_type, buf.into()))
                .map_err(GitOpsError::SendError)?;
        }
        Ok::<(), GitOpsError>(())
    })
}

fn run_action(
    name: &str,
    action: &Action,
    cwd: &Path,
    deadline: Instant,
    tx: &Sender<ActionOutput>,
) -> Result<(), GitOpsError> {
    let mut command = build_command(action, cwd);
    let mut child = command.spawn().map_err(GitOpsError::ActionError)?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    // TODO Proper cleanup; break read threads, et c
    emit_data(name.to_string(), stdout, SourceType::StdOut, tx.clone());
    emit_data(name.to_string(), stderr, SourceType::StdErr, tx.clone());
    loop {
        if let Some(exit) = child.try_wait().map_err(GitOpsError::ActionError)? {
            tx.send(ActionOutput::Exit(name.to_string(), exit))
                .map_err(GitOpsError::SendError)?;
            break;
        }
        if Instant::now() > deadline {
            child.kill().map_err(GitOpsError::ActionError)?;
            tx.send(ActionOutput::Timeout(name.to_string()))
                .map_err(GitOpsError::SendError)?;
            break;
        }
        sleep(Duration::from_secs(1));
    }
    Ok(())
}

// TODO SSH support
// TODO branch support
fn fetch_repo(
    config: GitConfig,
    deadline: Instant,
    target: &Path,
) -> Result<ObjectId, GitOpsError> {
    let should_interrupt = AtomicBool::new(false);
    let cancel = AtomicBool::new(false);
    let repository = scope(|s| {
        s.spawn(|| {
            while Instant::now() < deadline && !cancel.load(Ordering::Relaxed) {
                sleep(Duration::from_secs(1));
            }
            should_interrupt.store(true, Ordering::Relaxed);
        });
        let progress = gix::progress::Discard;
        let (mut checkout, _) = gix::prepare_clone(config.url, target)
            .unwrap()
            .fetch_then_checkout(progress, &should_interrupt)
            .map_err(GitOpsError::InitRepo)?;
        let (repository, _) = checkout
            .main_worktree(Discard, &should_interrupt)
            .map_err(GitOpsError::CheckoutRepo)?;
        cancel.store(true, Ordering::Relaxed);
        Ok(repository)
    })?;
    Ok(repository.head_commit().map(|c| c.id).unwrap())
}

fn eligible_task(task: &Task) -> bool {
    task.worker.is_none() && task.state.next_run < SystemTime::now()
}

fn finished_task(task: &Task) -> bool {
    task.worker.as_ref().map(JoinHandle::is_finished).is_some()
}

fn start_task(
    task: &Task,
    tx: Sender<ActionOutput>,
) -> Result<JoinHandle<Result<ObjectId, GitOpsError>>, GitOpsError> {
    let config = task.config.clone();
    let current_sha = task.state.current_sha;
    let workdir = tempfile::tempdir()
        .map_err(GitOpsError::WorkDir)?
        .into_path();
    let deadline = Instant::now() + config.timeout;
    let worker = spawn(move || {
        let new_sha = fetch_repo(config.git, deadline, &workdir)?;
        if current_sha != new_sha {
            tx.send(ActionOutput::Changes(
                config.name.clone(),
                current_sha,
                new_sha,
            ))
            .map_err(GitOpsError::SendError)?;
            for action in config.actions {
                let name = format!("{}|{}", config.name, action.name);
                run_action(&name, &action, &workdir, deadline, &tx)?;
            }
        }
        std::fs::remove_dir_all(workdir).map_err(GitOpsError::WorkDir)?;
        Ok(new_sha)
    });
    Ok(worker)
}

fn run<F>(
    tasks: &mut [Task],
    mut persist: F,
    tx: &Sender<ActionOutput>,
) -> Result<Infallible, GitOpsError>
where
    F: FnMut(&Task) -> Result<(), GitOpsError>,
{
    loop {
        if let Some(task) = tasks.iter_mut().find(|t| eligible_task(t)) {
            let task_tx = tx.clone();
            task.worker = Some(start_task(task, task_tx)?);
            task.state.next_run = SystemTime::now().add(task.config.interval);
            persist(task)?;
            continue;
        }
        if let Some(mut task) = tasks.iter_mut().find(|t| finished_task(t)) {
            let worker = task.worker.take().unwrap();
            match worker.join().unwrap() {
                Ok(new_sha) => {
                    task.state.current_sha = new_sha;
                    persist(task)?;
                }
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
            ActionOutput::Changes(name, prev_sha, new_sha) => {
                if prev_sha == ObjectId::null(Kind::Sha1) {
                    println!("{}: New repo @ {}", name, new_sha);
                } else {
                    println!("{}: Updated repo {} -> {}", name, prev_sha, new_sha);
                }
            }
            ActionOutput::Output(name, source_type, data) => match source_type {
                SourceType::StdOut => println!("{}: {}", name, String::from_utf8_lossy(&data)),
                SourceType::StdErr => eprintln!("{}: {}", name, String::from_utf8_lossy(&data)),
            },
            ActionOutput::Exit(name, exit) => println!("{}: exited with code {}", name, exit),
            ActionOutput::Timeout(name) => println!("{}: took too long", name),
        }
    }
}

#[derive(Parser)]
struct CliOptions {
    /// Path where state is stored
    #[clap(long)]
    state_file: Option<PathBuf>,
    /// YAML format task descriptions
    #[clap(long)]
    config_file: Option<String>,
    /// Git repository URL (http(s) for now)
    #[clap(long)]
    url: Option<String>,
    // /// Branch to check out
    // #[clap(long)]
    // branch: String,
    /// Command to execute on change (passed to /bin/sh)
    #[clap(long)]
    action: Option<String>,
    /// Environment variable for action
    #[clap(long)]
    environment: Vec<String>,
    /// Check repo for changes at this interval
    #[clap(long)]
    interval: Option<f32>,
    /// Max run time for repo fetch plus action in seconds
    #[clap(long)]
    timeout: Option<f32>,
}

fn tasks_from_file(opts: &CliOptions) -> Result<Vec<Task>, GitOpsError> {
    let config =
        File::open(opts.config_file.clone().unwrap()).map_err(GitOpsError::MissingConfig)?;
    let config_file: ConfigFile =
        serde_yaml::from_reader(config).map_err(GitOpsError::MalformedConfig)?;
    Ok(config_file
        .tasks
        .into_iter()
        .map(|c| Task {
            config: c,
            state: State::default(),
            worker: None,
        })
        .collect())
}

fn tasks_from_opts(opts: &CliOptions) -> Result<Vec<Task>, GitOpsError> {
    let url = Url::try_from(opts.url.clone().unwrap()).map_err(GitOpsError::InvalidUrl)?;
    let mut environment = HashMap::new();
    for env in &opts.environment {
        let (key, val) = env
            .split_once('=')
            .ok_or_else(|| GitOpsError::InvalidEnvVar(env.clone()))?;
        environment.insert(key.to_owned(), val.to_owned());
    }
    let action = Action {
        name: opts.action.clone().unwrap(),
        entrypoint: "/bin/sh".to_string(),
        args: vec!["-c".to_string(), opts.action.clone().unwrap()],
        environment,
        inherit_environment: false,
    };
    let actions = vec![action];
    Ok(vec![Task {
        config: GitOpsConfig {
            name: url.path.to_string(),
            git: GitConfig {
                url, /* branch: opts.branch.clone() */
            },
            actions,
            interval: opts
                .interval
                .map_or(default_interval(), Duration::from_secs_f32),
            timeout: opts
                .timeout
                .map_or(default_timeout(), Duration::from_secs_f32),
        },
        state: State::default(),
        worker: None,
    }])
}

trait Store {
    fn get(&self, id: &str) -> Option<&State>;
    fn retain(&mut self, task_ids: HashSet<String>);
    fn persist(&mut self, task: &Task) -> Result<(), GitOpsError>;
}

#[derive(Debug, Default)]
struct FileStore {
    path: PathBuf,
    state: HashMap<String, State>,
}

impl FileStore {
    fn from_file(path: PathBuf) -> Result<Self, GitOpsError> {
        let state = if path.try_exists().map_err(GitOpsError::StateFile)? {
            let file = File::open(&path).map_err(GitOpsError::LoadingState)?;
            serde_yaml::from_reader(file).map_err(GitOpsError::SerdeState)?
        } else {
            HashMap::new()
        };
        Ok(FileStore { path, state })
    }
}

impl Store for FileStore {
    fn get(&self, id: &str) -> Option<&State> {
        self.state.get(id)
    }

    fn retain(&mut self, task_ids: HashSet<String>) {
        self.state.retain(|id, _| task_ids.contains(id));
    }

    fn persist(&mut self, task: &Task) -> Result<(), GitOpsError> {
        self.state.insert(task.id(), task.state.clone());
        let file = File::create(&self.path).map_err(GitOpsError::SavingState)?;
        serde_yaml::to_writer(file, &self.state).map_err(GitOpsError::SerdeState)
    }
}

fn main() -> Result<Infallible, GitOpsError> {
    let opts = CliOptions::parse();
    let mut tasks = if opts.action.is_some() || opts.url.is_some() {
        if opts.action.is_none() || opts.url.is_none() || opts.config_file.is_some() {
            return Err(GitOpsError::ConfigConflict);
        }
        tasks_from_opts(&opts)?
    } else {
        if opts.config_file.is_none() {
            return Err(GitOpsError::ConfigConflict);
        }
        tasks_from_file(&opts)?
    };
    let (tx, rx) = channel();
    // TODO Handle TERM both here and when running actions
    spawn(move || {
        logging_receiver(&rx);
    });
    let state_path = opts
        .state_file
        .unwrap_or_else(|| PathBuf::from("./state.yaml".to_string()));
    let mut store = FileStore::from_file(state_path)?;
    let task_ids = tasks.iter().map(Task::id).collect::<HashSet<_>>();
    store.retain(task_ids);
    for task in &mut tasks {
        if let Some(s) = store.get(&task.id()) {
            task.state = s.clone();
        }
    }
    run(&mut tasks, |t: &Task| store.persist(t), &tx)
}
