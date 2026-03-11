use crate::commands::Outputs;
use crate::config::{ConfigManager, InstanceMetadata};
use crate::docker::DockerManager;
use crate::CommandOutput;
use anyhow::Result;
use serde::Serialize;

#[derive(Serialize)]
pub struct ListOutput {
    pub instances: Vec<InstanceStatus>,
}

#[derive(Serialize)]
pub struct InstanceStatus {
    pub metadata: InstanceMetadata,
    pub status: String,
}

impl CommandOutput for ListOutput {
    fn to_text(&self) -> String {
        let mut out = format!("{:<15} {:<10} {:<10} {:<10}\n", "NAME", "VERSION", "PORT", "STATUS");
        for inst in &self.instances {
            out.push_str(&format!(
                "{:<15} {:<10} {:<10} {:<10}\n",
                inst.metadata.name, inst.metadata.version, inst.metadata.port, inst.status
            ));
        }
        out
    }
}

pub async fn list_instances(
    config_mgr: &ConfigManager,
    docker_mgr: &DockerManager,
) -> Result<Outputs> {
    let config = config_mgr.load_config()?;
    let docker_instances = docker_mgr.list_instances().await?;

    let mut instances = Vec::new();
    for (name, metadata) in config.instances {
        let status = docker_instances
            .iter()
            .find(|(n, _)| n == &name)
            .map(|(_, s)| s.as_str())
            .unwrap_or("not found")
            .to_string();
        instances.push(InstanceStatus { metadata, status });
    }
    Ok(Outputs::List(ListOutput { instances }))
}
