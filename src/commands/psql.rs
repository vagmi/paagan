use crate::commands::Outputs;
use crate::docker::DockerManager;
use crate::CommandOutput;
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

pub async fn connect_psql(docker_mgr: &DockerManager, name: String) -> Result<Outputs> {
    docker_mgr.exec_psql(&name).await?;
    Ok(Outputs::Psql( PsqlOutput { name }))
}
