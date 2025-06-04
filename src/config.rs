use anyhow::{Context, Result};
use serde::Deserialize;
use std::{path::PathBuf, sync::OnceLock};
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
    // Whether to enable the stdout reporter.
    #[serde(default = "ReporterConfig::default_enable_stdout")]
    pub enable_stdout: bool,
}

impl ReporterConfig {
    // Default setting for enabling the stdout reporter (false).
    fn default_enable_stdout() -> bool {
        false
    }
}

impl Default for ReporterConfig {
    fn default() -> Self {
        Self {
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

// Use OnceLock for lazy, thread-safe initialization of the static config.
static INSTANCE: OnceLock<Config> = OnceLock::new();

impl Config {
    /// Initializes the global configuration instance from a TOML file.
    ///
    /// This function should be called once, typically at application startup.
    /// It attempts to load the configuration from the specified `config_path`.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The configuration file cannot be read or parsed.
    /// - The configuration has already been initialized (e.g., by a previous call).
    pub fn init(config_path: &PathBuf) -> Result<(), anyhow::Error> {
        match Self::load_from_file(config_path) {
            Ok(config) => {
                tracing::debug!("Configuration: {:?}", config);
                // Attempt to set the loaded config.
                if INSTANCE.set(config).is_err() {
                    // This happens if init() was called more than once.
                    anyhow::bail!("Configuration has already been initialized.");
                }
                Ok(())
            }
            Err(e) => {
                // If loading from file failed, propagate the error.
                // Do not set default config here.
                Err(e).context(format!(
                    "Failed to initialize configuration from {}",
                    config_path.display()
                ))
            }
        }
    }

    // Loads configuration from a TOML file.
    fn load_from_file(config_path: &PathBuf) -> Result<Self> {
        let content = std::fs::read_to_string(config_path)?;
        toml::from_str::<Config>(&content)
            .with_context(|| format!("Failed to parse config file: {}", config_path.display()))
    }
}

/// Returns a static reference to the global configuration instance.
///
/// Panics if the configuration hasn't been initialized by calling `Config::init()` first.
pub fn instance() -> &'static Config {
    INSTANCE
        .get()
        .expect("Configuration has not been initialized. Call Config::init() first.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    // Helper function to create a temporary config file with given content.
    fn create_temp_config(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        file.write_all(content.as_bytes())
            .expect("Failed to write to temp file");
        file
    }

    #[test]
    fn test_load_valid_config() {
        let config_content = r#"
[dispatcher]
check_interval = "10s"

[worker]
num_instance = 5

[reporter]
enable_stdout = true
"#;
        let temp_file = create_temp_config(config_content);
        let config_path = temp_file.path().to_path_buf();

        let config_result = Config::load_from_file(&config_path);
        assert!(config_result.is_ok());
        let config = config_result.unwrap();

        assert_eq!(config.dispatcher.check_interval, Duration::from_secs(10));
        assert_eq!(config.worker.num_instance, 5);
        assert!(config.reporter.enable_stdout);
    }

    #[test]
    fn test_load_partial_config_uses_defaults() {
        let config_content = r#"
[worker]
num_instance = 20
"#;
        let temp_file = create_temp_config(config_content);
        let config_path = temp_file.path().to_path_buf();

        let config_result = Config::load_from_file(&config_path);
        assert!(config_result.is_ok());
        let config = config_result.unwrap();

        // Check specified value
        assert_eq!(config.worker.num_instance, 20);

        // Check default values for missing sections/fields
        assert_eq!(config.dispatcher.check_interval, DispatcherConfig::default_check_interval());
        assert_eq!(config.reporter.enable_stdout, ReporterConfig::default_enable_stdout());
    }

    #[test]
    fn test_load_empty_config_uses_defaults() {
        let config_content = ""; // Empty config file
        let temp_file = create_temp_config(config_content);
        let config_path = temp_file.path().to_path_buf();

        let config_result = Config::load_from_file(&config_path);
        assert!(config_result.is_ok());
        let config = config_result.unwrap();

        // Check all default values individually since Config::default() is removed
        assert_eq!(config.dispatcher.check_interval, DispatcherConfig::default_check_interval());
        assert_eq!(config.worker.num_instance, WorkerConfig::default_num_instance());
        assert_eq!(config.reporter.enable_stdout, ReporterConfig::default_enable_stdout());
    }

    #[test]
    fn test_load_invalid_toml() {
        let config_content = r#"
[dispatcher
check_interval = "10s" # Missing closing bracket
"#;
        let temp_file = create_temp_config(config_content);
        let config_path = temp_file.path().to_path_buf();

        let config_result = Config::load_from_file(&config_path);
        assert!(config_result.is_err());
        let err = config_result.unwrap_err();
        // Check that the error is caused by toml parse error
        let found = err.chain().any(|e| e.is::<toml::de::Error>());
        assert!(found, "Error should be toml::de::Error");
    }

    #[test]
    fn test_load_non_existent_file() {
        let config_path = PathBuf::from("non_existent_config_file.toml");
        let config_result = Config::load_from_file(&config_path);
        assert!(config_result.is_err());
        let err = config_result.unwrap_err();
        // Check that the error is caused by std::io::ErrorKind::NotFound
        let io_err = err
            .downcast_ref::<std::io::Error>()
            .expect("Error should be std::io::Error");
        assert_eq!(io_err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn test_load_config_with_invalid_duration() {
        let config_content = r#"
[dispatcher]
check_interval = "5xyz" # Invalid duration format
"#;
        let temp_file = create_temp_config(config_content);
        let config_path = temp_file.path().to_path_buf();

        let config_result = Config::load_from_file(&config_path);
        assert!(config_result.is_err());
        let err = config_result.unwrap_err();
        // Check that the error is caused by toml parse error
        let found = err.chain().any(|e| e.is::<toml::de::Error>());
        assert!(found, "Error should be toml::de::Error");
    }

    #[test]
    fn test_load_config_with_wrong_type() {
        let config_content = r#"
 [worker]
 num_instance = "not a number"
 "#;
        let temp_file = create_temp_config(config_content);
        let config_path = temp_file.path().to_path_buf();

        let config_result = Config::load_from_file(&config_path);
        assert!(config_result.is_err());
        let err = config_result.unwrap_err();
        // Check that the error is caused by toml parse error
        let found = err.chain().any(|e| e.is::<toml::de::Error>());
        assert!(found, "Error should be toml::de::Error");
    }

    // Note: Testing `init()` and `instance()` directly is complex due to the static nature
    // of `INSTANCE: OnceLock`. The state persists across tests run in the same process.
    // Testing `load_from_file` covers the core logic of reading and parsing the config.
    // After a successful call to `init()` in your application's main function,
    // `instance()` should return the loaded configuration or panic if `init()` failed or wasn't called.
}
