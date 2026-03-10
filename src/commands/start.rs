use crate::config::ConfigManager;
use crate::docker::DockerManager;
use anyhow::Result;

pub async fn start_instance(
    config_mgr: &ConfigManager,
    docker_mgr: &DockerManager,
    name: String,
) -> Result<()> {
    let metadata = config_mgr.get_instance(&name)?;
    let docker_instances = docker_mgr.list_instances().await?;
    
    let is_running = docker_instances.iter()
        .any(|(n, s)| n == &name && s == "running");
    
    if is_running {
        println!("Instance '{}' is already running.", name);
        return Ok(());
    }

    let exists = docker_instances.iter().any(|(n, _)| n == &name);
    
    if exists {
        println!("Starting existing container for '{}'...", name);
        docker_mgr.start_container(&name).await?;
    } else {
        println!("Container for '{}' not found, recreating...", name);
        let instance_dir = config_mgr.get_instance_dir(&name);
        let data_dir = instance_dir.join("data").to_string_lossy().to_string();
        let archive_dir = instance_dir.join("archive").to_string_lossy().to_string();
        let backup_dir = instance_dir.join("backups").to_string_lossy().to_string();

        docker_mgr.create_instance_container(
            &name,
            &metadata.version,
            metadata.port,
            &data_dir,
            &archive_dir,
            &backup_dir,
            None,
        ).await?;
    }

    println!("Instance '{}' started on port {}.", name, metadata.port);
    Ok(())
}
