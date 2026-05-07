use crate::CommandOutput;
use crate::commands::Outputs;
use crate::config::{ConfigManager, InitMode};
use anyhow::Result;
use serde::Serialize;

#[derive(Serialize)]
pub struct ShowOutput {
    pub name: String,
    pub version: String,
    pub port: u16,
    pub connection_string: String,
    pub data_directory: String,
    pub image: String,
    pub init_mode: String,
    pub shared_preload_libraries: Option<String>,
}

impl CommandOutput for ShowOutput {
    fn to_text(&self) -> String {
        let mut out = format!(
            "Instance: {}\nVersion: {}\nImage: {}\nInit mode: {}\nPort: {}\nConnection string: {}\nData directory: {}",
            self.name,
            self.version,
            self.image,
            self.init_mode,
            self.port,
            self.connection_string,
            self.data_directory
        );
        if let Some(spls) = &self.shared_preload_libraries {
            out.push_str(&format!("\nshared_preload_libraries: {}", spls));
        }
        out
    }
}

pub async fn show_instance(config_mgr: &ConfigManager, name: String) -> Result<Outputs> {
    let metadata = config_mgr.get_instance(&name)?;
    let connection_string = format!("postgresql://postgres@localhost:{}/postgres", metadata.port);
    let data_directory = config_mgr
        .get_instance_dir(&name)
        .join("data")
        .to_string_lossy()
        .to_string();
    let image = metadata.resolved_image();
    let init_mode = match metadata.init_mode {
        InitMode::Standard => "standard",
        InitMode::Cnpg => "cnpg",
    }
    .to_string();

    Ok(Outputs::Show(ShowOutput {
        name: metadata.name,
        version: metadata.version,
        port: metadata.port,
        connection_string,
        data_directory,
        image,
        init_mode,
        shared_preload_libraries: metadata.shared_preload_libraries,
    }))
}
