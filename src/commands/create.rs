use crate::commands::Outputs;
use crate::config::{ConfigManager, InstanceMetadata};
use crate::docker::DockerManager;
use crate::CommandOutput;
use anyhow::{Context, Result};
use portpicker::pick_unused_port;
use serde::Serialize;

#[derive(Serialize)]
pub struct CreateOutput {
    pub name: String,
    pub port: u16,
    pub connection_string: String,
}

impl CommandOutput for CreateOutput {
    fn to_text(&self) -> String {
        format!(
            "Created instance '{}' on port {}\nConnection string: {}",
            self.name, self.port, self.connection_string
        )
    }
}

pub async fn create_instance(
    config_mgr: &ConfigManager,
    docker_mgr: &DockerManager,
    version: String,
    name: String,
) -> Result<Outputs> {
    let port = pick_unused_port().context("No available ports")?;
    config_mgr.create_instance_dirs(&name).await?;

    let instance_dir = config_mgr.get_instance_dir(&name);
    let data_dir = instance_dir.join("data").to_string_lossy().to_string();
    let archive_dir = instance_dir.join("archive").to_string_lossy().to_string();
    let backup_dir = instance_dir.join("backups").to_string_lossy().to_string();

    let container_id = docker_mgr
        .create_instance_container(
            &name,
            &version,
            port,
            &data_dir,
            &archive_dir,
            &backup_dir,
            None,
        )
        .await?;

    let (_, data_subdir) = docker_mgr.get_mount_info(&version);

    let metadata = InstanceMetadata {
        name: name.clone(),
        version,
        port,
        container_id: Some(container_id),
        data_subdir,
    };
    config_mgr.add_instance(metadata)?;

    let connection_string = format!("postgresql://postgres@localhost:{}/postgres", port);
    eprintln!("Created instance '{}' on port {}", name, port);
    eprintln!("Connection string: {}", connection_string);

    // Trigger initial baseline backup to enable PITR from the start
    eprintln!("Waiting for database to be ready for initial backup...");
    // Give some time for post-bootstrap initialization
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;

    eprintln!("Taking initial baseline backup...");
    let backup_snapshot_dir = "/backups/base_snapshot";
    if let Err(e) = docker_mgr.run_basebackup(&name, backup_snapshot_dir).await {
        eprintln!(
            "Warning: Initial backup failed: {}. PITR will require a manual backup first.",
            e
        );
    } else {
        eprintln!("Initial baseline backup completed successfully.");
    }

    Ok(Outputs::Create(CreateOutput {
        name,
        port,
        connection_string,
    }))
}
