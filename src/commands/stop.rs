use crate::commands::Outputs;
use crate::docker::DockerManager;
use crate::CommandOutput;
use anyhow::Result;
use serde::Serialize;

#[derive(Serialize)]
pub struct StopOutput {
    pub name: String,
}

impl CommandOutput for StopOutput {
    fn to_text(&self) -> String {
        format!("Instance '{}' stopped.", self.name)
    }
}

pub async fn stop_instance(docker_mgr: &DockerManager, name: String) -> Result<Outputs> {
    eprintln!("Stopping instance '{}'...", name);
    docker_mgr.stop_container(&name).await?;
    eprintln!("Instance '{}' stopped.", name);
    Ok(Outputs::Stop(StopOutput { name }))
}
