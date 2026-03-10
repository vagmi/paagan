use crate::config::ConfigManager;
use crate::docker::DockerManager;
use anyhow::Result;

pub async fn delete_instance(
    config_mgr: &ConfigManager,
    docker_mgr: &DockerManager,
    name: String,
    force: bool,
) -> Result<()> {
    let metadata = config_mgr.get_instance(&name)?;
    
    if !force {
        println!("Are you sure you want to delete instance '{}' (port {})? [y/N]", name, metadata.port);
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if input.trim().to_lowercase() != "y" {
            println!("Aborted.");
            return Ok(());
        }
    }
    
    println!("Stopping and removing container for '{}'...", name);
    if let Err(e) = docker_mgr.delete_container(&name).await {
        eprintln!("Warning: could not remove docker container: {}", e);
    }
    
    println!("Removing data and configuration for '{}'...", name);
    config_mgr.remove_instance(&name)?;
    
    println!("Instance '{}' deleted successfully.", name);
    Ok(())
}
