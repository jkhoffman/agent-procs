use tempfile::TempDir;

pub struct TestContext {
    pub state_dir: TempDir,
    pub session: String,
}

impl TestContext {
    pub fn new(session: &str) -> Self {
        Self {
            state_dir: TempDir::new().unwrap(),
            session: session.into(),
        }
    }

    pub fn set_env(&self) {
        std::env::set_var("XDG_STATE_HOME", self.state_dir.path());
    }
}
