use crate::docker::DockerManager;
use anyhow::Result;

pub async fn stop_instance(docker_mgr: &DockerManager, name: String) -> Result<()> {
    println!("Stopping instance '{}'...", name);
    docker_mgr.stop_container(&name).await?;
    println!("Instance '{}' stopped.", name);
    Ok(())
}
