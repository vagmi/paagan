use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InstanceMetadata {
    pub name: String,
    pub version: String,
    pub port: u16,
    pub container_id: Option<String>,
    pub data_subdir: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Config {
    pub instances: HashMap<String, InstanceMetadata>,
}

pub struct ConfigManager {
    base_dir: PathBuf,
}

impl ConfigManager {
    pub fn new() -> Result<Self> {
        let home_dir = dirs::home_dir().context("Could not find home directory")?;
        let base_dir = home_dir.join(".paagan");

        if !base_dir.exists() {
            // Use shell to create to ensure permissions as per user suggestion
            std::process::Command::new("mkdir")
                .arg("-p")
                .arg(&base_dir)
                .status()?;

            std::process::Command::new("mkdir")
                .arg("-p")
                .arg(base_dir.join("instances"))
                .status()?;
        }

        Ok(Self { base_dir })
    }

    pub fn get_instance_dir(&self, name: &str) -> PathBuf {
        self.base_dir.join("instances").join(name)
    }

    pub fn create_instance_dirs(&self, name: &str) -> Result<()> {
        let dir = self.get_instance_dir(name);

        // Use shell out as suggested by user
        std::process::Command::new("mkdir")
            .arg("-p")
            .arg(dir.join("data"))
            .status()?;

        std::process::Command::new("mkdir")
            .arg("-p")
            .arg(dir.join("archive"))
            .status()?;

        std::process::Command::new("mkdir")
            .arg("-p")
            .arg(dir.join("backups"))
            .status()?;

        Ok(())
    }

    pub fn load_config(&self) -> Result<Config> {
        let config_path = self.base_dir.join("instances.json");
        if !config_path.exists() {
            return Ok(Config::default());
        }
        let content = fs::read_to_string(config_path)?;
        let config = serde_json::from_str(&content)?;
        Ok(config)
    }

    pub fn save_config(&self, config: &Config) -> Result<()> {
        let config_path = self.base_dir.join("instances.json");
        let content = serde_json::to_string_pretty(config)?;
        fs::write(config_path, content)?;
        Ok(())
    }

    pub fn add_instance(&self, metadata: InstanceMetadata) -> Result<()> {
        let mut config = self.load_config()?;
        config.instances.insert(metadata.name.clone(), metadata);
        self.save_config(&config)
    }

    pub fn get_instance(&self, name: &str) -> Result<InstanceMetadata> {
        let config = self.load_config()?;
        config
            .instances
            .get(name)
            .cloned()
            .context(format!("Instance '{}' not found", name))
    }

    pub fn remove_instance(&self, name: &str) -> Result<()> {
        let mut config = self.load_config()?;
        config.instances.remove(name);
        self.save_config(&config)?;

        let dir = self.get_instance_dir(name);
        if dir.exists() {
            fs::remove_dir_all(dir)?;
        }
        Ok(())
    }
}
