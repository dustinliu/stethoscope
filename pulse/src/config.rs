use anyhow::Result;
use serde::Deserialize;
use std::sync::OnceLock;
use tokio::time::Duration;

// Parses a duration string (e.g., "5s", "1m") into a `tokio::time::Duration`.
// Used for deserializing duration values from the config file.
fn parse_duration<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    humantime::parse_duration(&s).map_err(serde::de::Error::custom)
}

/// Configuration specific to the Dispatcher component.
/// Corresponds to the [dispatcher] section in the TOML config file.
#[derive(Debug, Deserialize, Clone)]
pub struct DispatcherConfig {
    // How often the dispatcher checks for new endpoints or updates.
    #[serde(
        default = "DispatcherConfig::default_check_interval",
        deserialize_with = "parse_duration"
    )]
    pub check_interval: Duration,
}

impl DispatcherConfig {
    // Default check interval for the dispatcher (5 minutes).
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

/// Configuration specific to the Worker components.
/// Corresponds to the [worker] section in the TOML config file.
#[derive(Debug, Deserialize, Clone)]
pub struct WorkerConfig {
    // The number of worker instances to spawn.
    #[serde(default = "WorkerConfig::default_num_instance")]
    pub num_instance: usize,
}

impl WorkerConfig {
    // Default number of worker instances (10).
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

/// Configuration specific to the Reporter components.
/// Corresponds to the [reporter] section in the TOML config file.
#[derive(Debug, Deserialize, Clone)]
pub struct ReporterConfig {
    // The buffer size for the alert broadcast channel.
    #[serde(default = "ReporterConfig::default_alert_buffer_len")]
    pub alert_buffer_len: usize,

    // Whether to enable the stdout reporter.
    #[serde(default = "ReporterConfig::default_enable_stdout")]
    pub enable_stdout: bool,
}

impl ReporterConfig {
    // Default alert buffer length (20).
    fn default_alert_buffer_len() -> usize {
        20
    }
    // Default setting for enabling the stdout reporter (false).
    fn default_enable_stdout() -> bool {
        false
    }
}

impl Default for ReporterConfig {
    fn default() -> Self {
        Self {
            alert_buffer_len: Self::default_alert_buffer_len(),
            enable_stdout: Self::default_enable_stdout(),
        }
    }
}

/// Represents the overall application configuration, loaded from a TOML file.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct Config {
    // Dispatcher specific configuration.
    #[serde(default)]
    pub dispatcher: DispatcherConfig,

    // Worker specific configuration.
    #[serde(default)]
    pub worker: WorkerConfig,

    // Reporter specific configuration.
    #[serde(default)]
    pub reporter: ReporterConfig,
}

// Global static instance of the configuration, loaded once.
static INSTANCE: OnceLock<Config> = OnceLock::new();

impl Config {
    // Attempts to load the configuration from a TOML file.
    // Searches for the file in a predefined list of candidate paths.
    // If no file is found or parsing fails, returns an error.
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

        tracing::warn!("No config file found or failed to parse, using default config");
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("No suitable config file found")))
    }
}

// Returns a static reference to the global configuration instance.
// If the configuration hasn't been loaded yet, it attempts to load it
// from a TOML file. If loading fails, it falls back to the default configuration.
pub fn instance() -> &'static Config {
    INSTANCE.get_or_init(|| {
        Config::from_toml_file("config.toml").unwrap_or_else(|e| {
            tracing::warn!("Failed to load config: {}. Using default config.", e);
            Config::default()
        })
    })
}
