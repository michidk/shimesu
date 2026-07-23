//! Configuration resolution with precedence: CLI flag > environment variable > config file > default.

use crate::error::{Result, ShimesuError};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigFile {
    #[serde(default)]
    pub installation: InstallationConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstallationConfig {
    pub stack_name: Option<String>,

    pub region: Option<String>,

    pub profile: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub stack_name: String,

    pub region: Option<String>,

    pub profile: Option<String>,

    pub json: bool,

    pub yes: bool,
}

impl Config {
    pub const DEFAULT_STACK_NAME: &'static str = "shimesu";

    pub fn load(cli: &crate::cli::Cli) -> Result<Self> {
        let file_config = load_config_file()?;
        Ok(resolve_config(cli, file_config))
    }
}

fn resolve_config(cli: &crate::cli::Cli, file_config: ConfigFile) -> Config {
    let stack_name = cli
        .stack
        .clone()
        .or_else(|| std::env::var("SHIMESU_STACK").ok())
        .or(file_config.installation.stack_name)
        .unwrap_or_else(|| Config::DEFAULT_STACK_NAME.to_string());

    let region = cli
        .region
        .clone()
        .or(file_config.installation.region);

    let profile = cli
        .profile
        .clone()
        .or_else(|| std::env::var("AWS_PROFILE").ok())
        .or(file_config.installation.profile);

    Config {
        stack_name,
        region,
        profile,
        json: cli.json,
        yes: cli.yes,
    }
}

pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|p| p.join("shimesu").join("config.toml"))
}

fn load_config_file() -> Result<ConfigFile> {
    let path = match config_path() {
        Some(path) => path,
        None => return Ok(ConfigFile::default()),
    };

    load_config_file_from_path(&path)
}

fn load_config_file_from_path(path: &std::path::Path) -> Result<ConfigFile> {
    if !path.exists() {
        return Ok(ConfigFile::default());
    }

    let contents = std::fs::read_to_string(path).map_err(|error| {
        ShimesuError::Config(format!(
            "Failed to read config file {}: {}",
            path.display(),
            error
        ))
    })?;

    let config: ConfigFile = toml::from_str(&contents).map_err(|error| {
        ShimesuError::Config(format!(
            "Failed to parse config file {}: {}",
            path.display(),
            error
        ))
    })?;

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands, SiteCommands};
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, original }
        }

        fn unset(key: &'static str) -> Self {
            let original = std::env::var(key).ok();
            unsafe {
                std::env::remove_var(key);
            }
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => unsafe {
                    std::env::set_var(self.key, value);
                },
                None => unsafe {
                    std::env::remove_var(self.key);
                },
            }
        }
    }

    fn test_cli() -> Cli {
        Cli {
            profile: None,
            region: None,
            json: false,
            yes: false,
            stack: None,
            command: Commands::Site(SiteCommands::List),
        }
    }

    #[test]
    fn test_default_config() {
        let config = ConfigFile::default();
        assert!(config.installation.stack_name.is_none());
        assert!(config.installation.region.is_none());
    }

    #[test]
    fn test_parse_config_toml() {
        let toml_str = r#"
[installation]
stack_name = "shimesu-prod"
region = "us-east-1"
profile = "myprofile"
"#;
        let config: ConfigFile = toml::from_str(toml_str).expect("config TOML should parse");
        assert_eq!(
            config.installation.stack_name,
            Some("shimesu-prod".to_string())
        );
        assert_eq!(config.installation.region, Some("us-east-1".to_string()));
        assert_eq!(config.installation.profile, Some("myprofile".to_string()));
    }

    #[test]
    fn test_parse_minimal_config() {
        let toml_str = r#"
[installation]
stack_name = "my-stack"
"#;
        let config: ConfigFile = toml::from_str(toml_str).expect("minimal TOML should parse");
        assert_eq!(config.installation.stack_name, Some("my-stack".to_string()));
        assert!(config.installation.region.is_none());
    }

    #[test]
    fn test_parse_empty_config() {
        let toml_str = "";
        let config: ConfigFile = toml::from_str(toml_str).expect("empty TOML should parse");
        assert!(config.installation.stack_name.is_none());
    }

    #[test]
    fn test_invalid_toml_returns_error() {
        let toml_str = "this is not valid toml [[[[";
        let result: std::result::Result<ConfigFile, _> = toml::from_str(toml_str);
        assert!(result.is_err());
    }

    #[test]
    fn test_config_path() {
        let path = config_path();
        if let Some(path) = path {
            assert!(path.to_string_lossy().contains("shimesu"));
            assert!(path.to_string_lossy().ends_with("config.toml"));
        }
    }

    #[test]
    fn test_missing_config_file_is_not_an_error() {
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        let missing_path = tempdir.path().join("config.toml");

        let config =
            load_config_file_from_path(&missing_path).expect("missing config should not error");

        assert!(config.installation.stack_name.is_none());
        assert!(config.installation.region.is_none());
        assert!(config.installation.profile.is_none());
    }

    #[test]
    fn test_cli_values_override_env_and_file_values() {
        let _env_guard = env_lock().lock().expect("env mutex should lock");
        let _stack = EnvGuard::set("SHIMESU_STACK", "env-stack");
        let _region = EnvGuard::set("AWS_REGION", "eu-west-1");
        let _profile = EnvGuard::set("AWS_PROFILE", "env-profile");

        let mut cli = test_cli();
        cli.stack = Some("cli-stack".to_string());
        cli.region = Some("ap-southeast-2".to_string());
        cli.profile = Some("cli-profile".to_string());
        cli.json = true;
        cli.yes = true;

        let config = resolve_config(
            &cli,
            ConfigFile {
                installation: InstallationConfig {
                    stack_name: Some("file-stack".to_string()),
                    region: Some("us-west-2".to_string()),
                    profile: Some("file-profile".to_string()),
                },
            },
        );

        assert_eq!(config.stack_name, "cli-stack");
        assert_eq!(config.region, Some("ap-southeast-2".to_string()));
        assert_eq!(config.profile.as_deref(), Some("cli-profile"));
        assert!(config.json);
        assert!(config.yes);
    }

    #[test]
    fn test_file_region_used_when_no_cli_override() {
        let _env_guard = env_lock().lock().expect("env mutex should lock");
        let _stack = EnvGuard::set("SHIMESU_STACK", "env-stack");
        let _profile = EnvGuard::set("AWS_PROFILE", "env-profile");

        let config = resolve_config(
            &test_cli(),
            ConfigFile {
                installation: InstallationConfig {
                    stack_name: Some("file-stack".to_string()),
                    region: Some("us-west-1".to_string()),
                    profile: Some("file-profile".to_string()),
                },
            },
        );

        assert_eq!(config.stack_name, "env-stack");
        assert_eq!(config.region, Some("us-west-1".to_string()));
        assert_eq!(config.profile.as_deref(), Some("env-profile"));
    }

    #[test]
    fn test_region_is_none_when_not_explicitly_set() {
        let _env_guard = env_lock().lock().expect("env mutex should lock");
        let _stack = EnvGuard::unset("SHIMESU_STACK");
        let _region = EnvGuard::unset("AWS_REGION");
        let _default_region = EnvGuard::unset("AWS_DEFAULT_REGION");
        let _profile = EnvGuard::unset("AWS_PROFILE");

        let config = resolve_config(&test_cli(), ConfigFile::default());

        assert_eq!(config.stack_name, Config::DEFAULT_STACK_NAME);
        assert!(config.region.is_none());
        assert!(config.profile.is_none());
        assert!(!config.json);
        assert!(!config.yes);
    }
}
