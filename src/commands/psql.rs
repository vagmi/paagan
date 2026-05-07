use crate::CommandOutput;
use crate::commands::Outputs;
use crate::config::ConfigManager;
use crate::docker::DockerManager;
use anyhow::Result;
use serde::Serialize;

#[derive(Serialize)]
pub struct PsqlOutput {
    pub name: String,
}

impl CommandOutput for PsqlOutput {
    fn to_text(&self) -> String {
        format!("psql session for '{}' ended.", self.name)
    }
}

pub async fn connect_psql(
    config_mgr: &ConfigManager,
    docker_mgr: &DockerManager,
    name: String,
) -> Result<Outputs> {
    let metadata = config_mgr.get_instance(&name)?;
    docker_mgr.exec_psql(&name, metadata.init_mode).await?;
    Ok(Outputs::Psql(PsqlOutput { name }))
}
