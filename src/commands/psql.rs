use crate::docker::DockerManager;
use anyhow::Result;

pub async fn connect_psql(docker_mgr: &DockerManager, name: String) -> Result<()> {
    docker_mgr.exec_psql(&name).await
}
