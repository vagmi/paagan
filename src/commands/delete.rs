use crate::commands::Outputs;
use crate::config::ConfigManager;
use crate::docker::DockerManager;
use crate::CommandOutput;
use anyhow::Result;
use serde::Serialize;

#[derive(Serialize)]
pub struct DeleteOutput {
    pub name: String,
    pub deleted: bool,
}

impl CommandOutput for DeleteOutput {
    fn to_text(&self) -> String {
        if self.deleted {
            format!("Instance '{}' deleted successfully.", self.name)
        } else {
            format!("Instance '{}' deletion aborted.", self.name)
        }
    }
}

pub async fn delete_instance(
    config_mgr: &ConfigManager,
    docker_mgr: &DockerManager,
    name: String,
    force: bool,
) -> Result<Outputs> {
    let metadata = config_mgr.get_instance(&name)?;

    if !force {
        eprintln!(
            "Are you sure you want to delete instance '{}' (port {})? [y/N]",
            name, metadata.port
        );
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if input.trim().to_lowercase() != "y" {
            eprintln!("Aborted.");
            return Ok(Outputs::Delete(DeleteOutput {
                name,
                deleted: false,
            }));
        }
    }

    eprintln!("Stopping and removing container for '{}'...", name);
    if let Err(e) = docker_mgr.delete_container(&name).await {
        eprintln!("Warning: could not remove docker container: {}", e);
    }

    eprintln!("Removing data and configuration for '{}'...", name);
    config_mgr.remove_instance(&name)?;

    eprintln!("Instance '{}' deleted successfully.", name);
    Ok( Outputs::Delete( DeleteOutput {
        name,
        deleted: true,
    }))
}
