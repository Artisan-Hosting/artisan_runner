use artisan_middleware::{
    config::AppConfig,
    version::{aml_version, str_to_version},
};
use colored::Colorize;
use config::{Config, ConfigError, File};
use dusa_collection_utils::{
    log::LogLevel, stringy::Stringy, types::PathType, version::{SoftwareVersion, Version, VersionCode},
};
use dusa_collection_utils::log;
use serde::Deserialize;
use std::fmt;

pub fn get_config() -> AppConfig {
    let mut config: AppConfig = match AppConfig::new() {
        Ok(loaded_data) => loaded_data,
        Err(e) => {
            log!(LogLevel::Error, "Couldn't load config: {}", e.to_string());
            std::process::exit(100)
        }
    };
    config.app_name = Stringy::from_string(env!("CARGO_PKG_NAME").to_string());

    let raw_version: SoftwareVersion = {
        // defining the version
        let library_version: Version = aml_version();
        let software_version: Version = str_to_version(env!("CARGO_PKG_VERSION"), Some(VersionCode::Production));
        
        SoftwareVersion {
            application: software_version,
            library: library_version,
        }
    };

    config.version = match serde_json::to_string(&raw_version) {
        Ok(ver) => ver,
        Err(err) => {
            log!(LogLevel::Error, "{}", err);
            std::process::exit(100);
        },
    };

    config.database = None;
    config
}

#[derive(Debug, Deserialize, Clone)]
pub struct AppSpecificConfig {
    pub interval_seconds: u32,
    pub monitor_path: String,
    pub project_path: String,
    pub changes_needed: i32,
    pub ignored_subdirs: Vec<String>, // Add ignored subdirectories as strings
}

#[allow(dead_code)]
impl AppSpecificConfig {
    pub fn safe_path(&self) -> PathType {
        let self_cloned = self.clone();
        let path = PathType::Content(self_cloned.monitor_path);
        if !path.exists() {
            log!(LogLevel::Error, "The path {} doesn't exist", path);
            std::process::exit(0)
        } else {
            match path.canonicalize() {
                Ok(canon_path) => PathType::PathBuf(canon_path),
                Err(e) => {
                    log!(
                        LogLevel::Error,
                        "Failed to canonicalize path: {}, using default: {}",
                        e,
                        path
                    );
                    path
                }
            }
        }
    }

    pub fn project_path(&self) -> PathType {
        let self_cloned = self.clone();
        let path = PathType::Content(self_cloned.project_path);
        if !path.exists() {
            log!(LogLevel::Error, "The path {} doesn't exist", path);
            std::process::exit(0)
        } else {
            match path.canonicalize() {
                Ok(canon_path) => PathType::PathBuf(canon_path),
                Err(e) => {
                    log!(
                        LogLevel::Error,
                        "Failed to canonicalize path: {}, using default: {}",
                        e,
                        path
                    );
                    path
                }
            }
        }
    }

    /// Converts ignored_subdirs strings into PathType objects relative to the monitor_path
    pub fn ignored_paths(&self) -> Option<Vec<PathType>> {
        let base_path = self.safe_path(); // Canonicalize the monitor path
        
        let sub_dirs: Vec<PathType> = self.ignored_subdirs
            .iter()
            .map(|subdir| PathType::PathBuf(base_path.join(subdir))) // Join each subdir to the base path
            .collect();

        if sub_dirs.is_empty() {
            return None
        }

        return Some(sub_dirs)
    }
}

pub fn specific_config() -> Result<AppSpecificConfig, ConfigError> {
    let mut builder = Config::builder();
    builder = builder.add_source(File::with_name("Config").required(false));

    let settings = builder.build()?;
    let app_specific: AppSpecificConfig = settings.get("app_specific")?;

    Ok(app_specific)
}

impl fmt::Display for AppSpecificConfig {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {{\n\
             \t{}: {},\n\
             \t{}: {},\n\
             \t{}: {},\n\
             \t{}: {},\n\
             \t{}: {},\n\
             }}",
            "AppSpecificConfig".cyan().bold(),
            "interval_seconds".yellow(),
            self.interval_seconds.to_string().green(),
            "monitor_path".yellow(),
            self.monitor_path.clone().green(),
            "project_path".yellow(),
            self.project_path.clone().green(),
            "changes_needed".yellow(),
            self.changes_needed.to_string().green(),
            "Ignored_directories".yellow(),
            self.ignored_subdirs.join(" ").green()
        )
    }
}
