use crate::CommandOutput;
use crate::commands::Outputs;
use crate::config::{ConfigManager, InitMode, InstanceMetadata};
use crate::docker::{ContainerSpec, DockerManager};
use anyhow::{Context, Result};
use portpicker::pick_unused_port;
use serde::Serialize;

pub struct CreateArgs {
    pub version: String,
    pub image: Option<String>,
    pub shared_preload_libraries: Option<String>,
    pub init_mode: InitMode,
    pub name: String,
}

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
    args: CreateArgs,
) -> Result<Outputs> {
    let CreateArgs {
        version,
        image,
        shared_preload_libraries,
        init_mode,
        name,
    } = args;

    let port = pick_unused_port().context("No available ports")?;
    config_mgr.create_instance_dirs(&name).await?;

    let instance_dir = config_mgr.get_instance_dir(&name);
    let data_dir = instance_dir.join("data").to_string_lossy().to_string();
    let archive_dir = instance_dir.join("archive").to_string_lossy().to_string();
    let backup_dir = instance_dir.join("backups").to_string_lossy().to_string();

    let resolved_image = image
        .clone()
        .unwrap_or_else(|| format!("postgres:{}", version));

    let spec = ContainerSpec {
        name: &name,
        image: &resolved_image,
        version: &version,
        init_mode,
        port,
        data_dir: &data_dir,
        archive_dir: &archive_dir,
        backup_dir: &backup_dir,
        restore_dir: None,
        shared_preload_libraries: shared_preload_libraries.as_deref(),
    };

    let container_id = docker_mgr.create_instance_container(&spec).await?;

    let (_, data_subdir) = docker_mgr.mount_info(init_mode, &version);

    let metadata = InstanceMetadata {
        name: name.clone(),
        version,
        port,
        container_id: Some(container_id),
        data_subdir,
        image,
        init_mode,
        shared_preload_libraries,
    };
    config_mgr.add_instance(metadata)?;

    let connection_string = format!("postgresql://postgres@localhost:{}/postgres", port);
    eprintln!("Created instance '{}' on port {}", name, port);
    eprintln!("Connection string: {}", connection_string);

    eprintln!("Waiting for database to be ready for initial backup...");
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;

    eprintln!("Taking initial baseline backup...");
    let backup_snapshot_dir = "/backups/base_snapshot";
    if let Err(e) = docker_mgr
        .run_basebackup(&name, backup_snapshot_dir, init_mode)
        .await
    {
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
