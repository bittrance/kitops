use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    thread::sleep,
    time::Duration,
};

use gix::{ObjectId, Url};

use crate::{
    errors::GitOpsError,
    git::UrlProvider,
    task::{scheduled::ScheduledTask, Workload},
};

impl<W: Workload + Clone + Send + 'static> ScheduledTask<W> {
    pub fn await_finished(&self) {
        while !self.is_finished() {
            sleep(Duration::from_millis(2));
        }
    }

    pub fn await_eligible(&self) {
        while !self.is_eligible() {
            sleep(Duration::from_millis(2));
        }
    }
}

pub struct TestUrl {
    url: Url,
    auth_url_error: Mutex<Option<GitOpsError>>,
}

impl TestUrl {
    pub fn new(auth_url_error: Option<GitOpsError>) -> Self {
        let url = gix::url::parse("https://example.com".into()).unwrap();
        TestUrl {
            url,
            auth_url_error: Mutex::new(auth_url_error),
        }
    }
}

impl UrlProvider for TestUrl {
    fn url(&self) -> &Url {
        &self.url
    }

    fn auth_url(&self) -> Result<gix::Url, crate::errors::GitOpsError> {
        if let Some(err) = self.auth_url_error.lock().unwrap().take() {
            Err(err)
        } else {
            Ok(self.url().clone())
        }
    }
}

#[derive(Clone, Default)]
pub struct TestWorkload {
    errfunc: Option<Arc<Box<dyn Fn() -> GitOpsError + Send + Sync>>>,
}

impl TestWorkload {
    pub fn fail_with(errfunc: impl Fn() -> GitOpsError + Send + Sync + 'static) -> Self {
        Self {
            errfunc: Some(Arc::new(Box::new(errfunc))),
            ..Default::default()
        }
    }
}

impl Workload for TestWorkload {
    fn id(&self) -> String {
        "test".to_string()
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(1)
    }

    fn perform(self, _workdir: PathBuf, _current_sha: ObjectId) -> Result<ObjectId, GitOpsError> {
        sleep(Duration::from_millis(10));
        if self.errfunc.is_some() {
            return Err(self.errfunc.unwrap()());
        }
        Ok(ObjectId::empty_blob(gix::hash::Kind::Sha1))
    }
}
