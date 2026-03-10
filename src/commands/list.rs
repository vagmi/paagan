use crate::config::ConfigManager;
use crate::docker::DockerManager;
use anyhow::Result;

pub async fn list_instances(config_mgr: &ConfigManager, docker_mgr: &DockerManager) -> Result<()> {
    let config = config_mgr.load_config()?;
    let docker_instances = docker_mgr.list_instances().await?;
    
    println!("{:<15} {:<10} {:<10} {:<10}", "NAME", "VERSION", "PORT", "STATUS");
    for (name, metadata) in config.instances {
        let status = docker_instances.iter()
            .find(|(n, _)| n == &name)
            .map(|(_, s)| s.as_str())
            .unwrap_or("not found");
        println!("{:<15} {:<10} {:<10} {:<10}", name, metadata.version, metadata.port, status);
    }
    Ok(())
}
