use std::{sync::OnceLock, time::Duration};

// Config struct definition
#[derive(Debug)]
pub struct Config {
    worker_num: usize,
    agent_num: usize,
    check_interval: Duration,
}

static INSTANCE: OnceLock<Config> = OnceLock::new();

impl Config {
    fn new() -> Self {
        Config {
            worker_num: 50,
            agent_num: 1,
            check_interval: Duration::from_secs(5),
        }
    }

    pub fn worker_num(&self) -> usize {
        self.worker_num
    }

    pub fn agent_num(&self) -> usize {
        self.agent_num
    }

    //TODO: change the default value to 5 minutes
    pub fn check_interval(&self) -> Duration {
        self.check_interval
    }
}

pub fn instance() -> &'static Config {
    INSTANCE.get_or_init(Config::new)
}
