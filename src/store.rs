use std::{
    collections::{HashMap, HashSet},
    fs::File,
    path::{Path, PathBuf},
};

use crate::{errors::GitOpsError, state::State, task::ScheduledTask, workload::Workload};

pub trait Store {
    fn get(&self, id: &str) -> Option<&State>;
    fn retain(&mut self, task_ids: HashSet<String>);
    fn persist<W: Workload + Clone + Send + 'static>(
        &mut self,
        id: String,
        task: &ScheduledTask<W>,
    ) -> Result<(), GitOpsError>;
}

#[derive(Debug, Default)]
pub struct FileStore {
    path: PathBuf,
    state: HashMap<String, State>,
}

impl FileStore {
    pub fn from_file(path: &Path) -> Result<Self, GitOpsError> {
        let state = if path.try_exists().map_err(GitOpsError::StateFile)? {
            let file = File::open(path).map_err(GitOpsError::LoadingState)?;
            serde_yaml::from_reader(file).map_err(GitOpsError::SerdeState)?
        } else {
            HashMap::new()
        };
        Ok(FileStore {
            path: path.to_path_buf(),
            state,
        })
    }
}

impl Store for FileStore {
    fn get(&self, id: &str) -> Option<&State> {
        self.state.get(id)
    }

    fn retain(&mut self, task_ids: HashSet<String>) {
        self.state.retain(|id, _| task_ids.contains(id));
    }

    fn persist<W: Workload + Clone + Send + 'static>(
        &mut self,
        id: String,
        task: &ScheduledTask<W>,
    ) -> Result<(), GitOpsError> {
        self.state.insert(id, task.state());
        let file = File::create(&self.path).map_err(GitOpsError::SavingState)?;
        serde_yaml::to_writer(file, &self.state).map_err(GitOpsError::SerdeState)
    }
}
