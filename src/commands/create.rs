use crate::config::{ConfigManager, InstanceMetadata};
use crate::docker::DockerManager;
use anyhow::{Context, Result};
use portpicker::pick_unused_port;

pub async fn create_instance(
    config_mgr: &ConfigManager,
    docker_mgr: &DockerManager,
    version: String,
    name: String,
) -> Result<()> {
    let port = pick_unused_port().context("No available ports")?;
    config_mgr.create_instance_dirs(&name)?;
    
    let instance_dir = config_mgr.get_instance_dir(&name);
    let data_dir = instance_dir.join("data").to_string_lossy().to_string();
    let archive_dir = instance_dir.join("archive").to_string_lossy().to_string();
    let backup_dir = instance_dir.join("backups").to_string_lossy().to_string();

    let container_id = docker_mgr.create_instance_container(
        &name,
        &version,
        port,
        &data_dir,
        &archive_dir,
        &backup_dir,
        None,
    ).await?;

    let (_, data_subdir) = docker_mgr.get_mount_info(&version);

    let metadata = InstanceMetadata {
        name: name.clone(),
        version,
        port,
        container_id: Some(container_id),
        data_subdir,
    };
    config_mgr.add_instance(metadata)?;
    
    println!("Created instance '{}' on port {}", name, port);
    println!("Connection string: postgresql://postgres@localhost:{}/postgres", port);

    // Trigger initial baseline backup to enable PITR from the start
    println!("Waiting for database to be ready for initial backup...");
    // Give some time for post-bootstrap initialization
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    
    println!("Taking initial baseline backup...");
    let backup_snapshot_dir = "/backups/base_snapshot";
    if let Err(e) = docker_mgr.run_basebackup(&name, backup_snapshot_dir).await {
        eprintln!("Warning: Initial backup failed: {}. PITR will require a manual backup first.", e);
    } else {
        println!("Initial baseline backup completed successfully.");
    }

    Ok(())
}
