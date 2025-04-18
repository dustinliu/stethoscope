use anyhow::Result;
use serde::Deserialize;
/// Configuration module for the Pulse URL monitoring system
///
/// This module manages the global configuration settings for the application,
/// including worker counts, agent counts, and timing intervals.
use std::sync::OnceLock;
use tokio::time::Duration;

fn parse_duration<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    humantime::parse_duration(&s).map_err(serde::de::Error::custom)
}

/// Dispatcher 設定，對應 [dispatcher] 區塊
#[derive(Debug, Deserialize, Clone)]
pub struct DispatcherConfig {
    #[serde(
        default = "DispatcherConfig::default_check_interval",
        deserialize_with = "parse_duration"
    )]
    pub check_interval: Duration,
}

impl DispatcherConfig {
    fn default_check_interval() -> Duration {
        Duration::from_secs(300)
    }
}

impl Default for DispatcherConfig {
    fn default() -> Self {
        Self {
            check_interval: Self::default_check_interval(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct WorkerConfig {
    #[serde(default = "WorkerConfig::default_num_instance")]
    pub num_instance: usize,
}

impl WorkerConfig {
    fn default_num_instance() -> usize {
        10
    }
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            num_instance: Self::default_num_instance(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ReporterConfig {
    #[serde(default = "ReporterConfig::default_alert_buffer_len")]
    pub alert_buffer_len: usize,

    #[serde(default = "ReporterConfig::default_enable_executer")]
    pub enable_stdout: bool,
}

impl ReporterConfig {
    fn default_alert_buffer_len() -> usize {
        20
    }
    fn default_enable_executer() -> bool {
        false
    }
}

impl Default for ReporterConfig {
    fn default() -> Self {
        Self {
            alert_buffer_len: Self::default_alert_buffer_len(),
            enable_stdout: Self::default_enable_executer(),
        }
    }
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct Config {
    #[serde(default = "Config::default_debug")]
    pub debug: bool,

    #[serde(default)]
    pub dispatcher: DispatcherConfig,

    #[serde(default)]
    pub worker: WorkerConfig,

    #[serde(default)]
    pub reporter: ReporterConfig,
}

static INSTANCE: OnceLock<Config> = OnceLock::new();

impl Config {
    pub fn from_toml_file(_unused: &str) -> anyhow::Result<Self> {
        // List of candidate config paths
        let candidates = [
            "./test.toml",
            "./pulse.toml",
            "./config/test.toml",
            "./config/pulse.toml",
            "./pulse/config/test.toml",
            "./pulse/config/pulse.toml",
        ];
        let mut last_err = None;
        for path in &candidates {
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(e) => {
                    last_err = Some(anyhow::anyhow!("Failed to read {}: {}", path, e));
                    continue;
                }
            };
            match toml::from_str::<Config>(&content) {
                Ok(config) => {
                    log::debug!("config: {:?}", config);
                    return Ok(config);
                }
                Err(e) => {
                    last_err = Some(anyhow::anyhow!("Failed to parse {}: {}", path, e));
                }
            }
        }

        log::warn!("No config file found, use default config");
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("No config file found")))
    }

    fn default_debug() -> bool {
        false
    }
}

pub fn instance() -> &'static Config {
    INSTANCE
        .get_or_init(|| Config::from_toml_file("config.toml").unwrap_or_else(|_| Config::default()))
}
