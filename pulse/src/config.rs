use anyhow::Result;
use serde::Deserialize;
use std::str::FromStr;
/// Configuration module for the Pulse URL monitoring system
///
/// This module manages the global configuration settings for the application,
/// including worker counts, agent counts, and timing intervals.
use std::sync::OnceLock;
use tokio::time::Duration;
use tracing::Level;

fn parse_duration<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    humantime::parse_duration(&s).map_err(serde::de::Error::custom)
}

fn parse_level<'de, D>(deserializer: D) -> Result<Level, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Level::from_str(&s).map_err(serde::de::Error::custom)
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

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    #[serde(
        default = "Config::default_log_level",
        deserialize_with = "parse_level"
    )]
    pub log_level: Level,

    #[serde(default)]
    pub dispatcher: DispatcherConfig,

    #[serde(default)]
    pub worker: WorkerConfig,

    #[serde(default)]
    pub reporter: ReporterConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            log_level: Self::default_log_level(),
            dispatcher: DispatcherConfig::default(),
            worker: WorkerConfig::default(),
            reporter: ReporterConfig::default(),
        }
    }
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
                    tracing::debug!("config: {:?}", config);
                    return Ok(config);
                }
                Err(e) => {
                    last_err = Some(anyhow::anyhow!("Failed to parse {}: {}", path, e));
                }
            }
        }

        tracing::warn!("No config file found, use default config");
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("No config file found")))
    }

    fn default_log_level() -> Level {
        Level::INFO
    }
}

pub fn instance() -> &'static Config {
    INSTANCE
        .get_or_init(|| Config::from_toml_file("config.toml").unwrap_or_else(|_| Config::default()))
}
