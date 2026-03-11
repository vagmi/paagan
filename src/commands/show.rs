use crate::commands::Outputs;
use crate::config::ConfigManager;
use crate::CommandOutput;
use anyhow::Result;
use serde::Serialize;

#[derive(Serialize)]
pub struct ShowOutput {
    pub name: String,
    pub version: String,
    pub port: u16,
    pub connection_string: String,
    pub data_directory: String,
}

impl CommandOutput for ShowOutput {
    fn to_text(&self) -> String {
        format!(
            "Instance: {}\nVersion: {}\nPort: {}\nConnection string: {}\nData directory: {}",
            self.name, self.version, self.port, self.connection_string, self.data_directory
        )
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

    Ok(Outputs::Show(ShowOutput {
        name: metadata.name,
        version: metadata.version,
        port: metadata.port,
        connection_string,
        data_directory,
    }))
}
