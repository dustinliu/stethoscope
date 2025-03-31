use std::sync::OnceLock;

// Config struct definition
#[derive(Debug)]
pub struct Config {
    worker_num: usize,
    agent_num: usize,
}

static INSTANCE: OnceLock<Config> = OnceLock::new();

impl Config {
    fn new() -> Self {
        Config {
            worker_num: 50,
            agent_num: 1,
        }
    }

    pub fn worker_num(&self) -> usize {
        self.worker_num
    }

    pub fn agent_num(&self) -> usize {
        self.agent_num
    }
}

pub fn instance() -> &'static Config {
    INSTANCE.get_or_init(Config::new)
}
