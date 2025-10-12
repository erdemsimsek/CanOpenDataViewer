use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub can_interface: String,
    pub node_id: u8,
    pub eds_file_path: Option<String>,
    pub enable_logging: bool,
    pub log_directory: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            can_interface: String::new(),
            node_id: 1,
            eds_file_path: None,
            enable_logging: true,
            log_directory: None,
        }
    }
}

impl AppConfig {
    /// Get the path to the config file
    pub fn config_file_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("com", "canopen", "canopen-viewer")
            .map(|proj_dirs| {
                let config_dir = proj_dirs.config_dir();
                config_dir.join("config.toml")
            })
    }

    /// Load configuration from file, returns default if file doesn't exist or on error
    pub fn load() -> Self {
        if let Some(config_path) = Self::config_file_path() {
            if config_path.exists() {
                match fs::read_to_string(&config_path) {
                    Ok(contents) => {
                        match toml::from_str(&contents) {
                            Ok(config) => {
                                println!("✓ Loaded configuration from {:?}", config_path);
                                return config;
                            }
                            Err(e) => {
                                eprintln!("Failed to parse config file: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to read config file: {}", e);
                    }
                }
            }
        }

        println!("Using default configuration");
        Self::default()
    }

    /// Save configuration to file
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(config_path) = Self::config_file_path() {
            // Create config directory if it doesn't exist
            if let Some(parent) = config_path.parent() {
                fs::create_dir_all(parent)?;
            }

            let toml_string = toml::to_string_pretty(self)?;
            fs::write(&config_path, toml_string)?;
            println!("✓ Saved configuration to {:?}", config_path);
            Ok(())
        } else {
            Err("Could not determine config file path".into())
        }
    }

    /// Get the default log directory path
    pub fn default_log_directory() -> Option<PathBuf> {
        directories::ProjectDirs::from("com", "canopen", "canopen-viewer")
            .map(|proj_dirs| {
                let data_dir = proj_dirs.data_local_dir();
                data_dir.join("logs")
            })
    }

    /// Get the log directory as PathBuf, using default if not set
    pub fn get_log_directory(&self) -> Option<PathBuf> {
        if let Some(ref dir) = self.log_directory {
            Some(PathBuf::from(dir))
        } else {
            Self::default_log_directory()
        }
    }
}
