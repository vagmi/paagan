use crate::CommandOutput;
use crate::commands::Outputs;
use crate::config::ConfigManager;
use crate::docker::{ContainerSpec, DockerManager};
use anyhow::Result;
use serde::Serialize;

#[derive(Serialize)]
pub struct StartOutput {
    pub name: String,
    pub port: u16,
}

impl CommandOutput for StartOutput {
    fn to_text(&self) -> String {
        format!("Instance '{}' started on port {}.", self.name, self.port)
    }
}

pub async fn start_instance(
    config_mgr: &ConfigManager,
    docker_mgr: &DockerManager,
    name: String,
) -> Result<Outputs> {
    let metadata = config_mgr.get_instance(&name)?;
    let docker_instances = docker_mgr.list_instances().await?;

    let is_running = docker_instances
        .iter()
        .any(|(n, s)| n == &name && s == "running");

    if is_running {
        eprintln!("Instance '{}' is already running.", name);
        return Ok(Outputs::Start(StartOutput {
            name,
            port: metadata.port,
        }));
    }

    let exists = docker_instances.iter().any(|(n, _)| n == &name);

    if exists {
        eprintln!("Starting existing container for '{}'...", name);
        docker_mgr.start_container(&name).await?;
    } else {
        eprintln!("Container for '{}' not found, recreating...", name);
        let instance_dir = config_mgr.get_instance_dir(&name);
        let data_dir = instance_dir.join("data").to_string_lossy().to_string();
        let archive_dir = instance_dir.join("archive").to_string_lossy().to_string();
        let backup_dir = instance_dir.join("backups").to_string_lossy().to_string();
        let resolved_image = metadata.resolved_image();

        let spec = ContainerSpec {
            name: &name,
            image: &resolved_image,
            version: &metadata.version,
            init_mode: metadata.init_mode,
            port: metadata.port,
            data_dir: &data_dir,
            archive_dir: &archive_dir,
            backup_dir: &backup_dir,
            restore_dir: None,
            shared_preload_libraries: metadata.shared_preload_libraries.as_deref(),
        };

        docker_mgr.create_instance_container(&spec).await?;
    }

    eprintln!("Instance '{}' started on port {}.", name, metadata.port);
    Ok(Outputs::Start(StartOutput {
        name,
        port: metadata.port,
    }))
}
