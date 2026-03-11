use bollard::container::{Config, CreateContainerOptions, StartContainerOptions, ListContainersOptions};
use bollard::models::HostConfig;
use bollard::Docker;
use anyhow::{Result, Context};
use std::collections::HashMap;

pub struct DockerManager {
    docker: Docker,
}

impl DockerManager {
    pub fn new() -> Result<Self> {
        let docker = Docker::connect_with_local_defaults().context("Failed to connect to Docker")?;
        Ok(Self { docker })
    }

    pub fn get_mount_info(&self, version: &str) -> (&'static str, Option<String>) {
        // Extract major version more robustly (handle e.g. "18-alpine")
        let major_str = version.split('.').next().unwrap_or("");
        // Take numeric prefix of major_str
        let major_prefix: String = major_str.chars().take_while(|c| c.is_ascii_digit()).collect();
        let major = major_prefix.parse::<u32>().unwrap_or(0);
        
        if major >= 18 || version == "latest" {
            let v = if version == "latest" { "18" } else { &major_prefix };
            ("/var/lib/postgresql", Some(format!("{}/docker", v)))
        } else {
            ("/var/lib/postgresql/data", None)
        }
    }

    pub async fn create_instance_container(
        &self,
        name: &str,
        version: &str,
        port: u16,
        data_dir: &str,
        archive_dir: &str,
        backup_dir: &str,
        restore_dir: Option<&str>,
    ) -> Result<String> {
        let image = format!("postgres:{}", version);
        let (mount_point, _) = self.get_mount_info(version);
        
        // Ensure image exists
        println!("Pulling image {}...", image);
        use futures_util::StreamExt;
        let mut pull_stream = self.docker.create_image(
            Some(bollard::image::CreateImageOptions {
                from_image: image.clone(),
                ..Default::default()
            }),
            None,
            None,
        );
        while let Some(pull_result) = pull_stream.next().await {
            let _ = pull_result.context("Failed to pull image")?;
        }

        let uid_output = std::process::Command::new("id").arg("-u").output()?;
        let gid_output = std::process::Command::new("id").arg("-g").output()?;
        let user_id = String::from_utf8_lossy(&uid_output.stdout).trim().to_string();
        let group_id = String::from_utf8_lossy(&gid_output.stdout).trim().to_string();
        let user_str = format!("{}:{}", user_id, group_id);
        
        let mut port_bindings = HashMap::new();
        port_bindings.insert(
            "5432/tcp".to_string(),
            Some(vec![bollard::models::PortBinding {
                host_ip: Some("0.0.0.0".to_string()),
                host_port: Some(port.to_string()),
            }]),
        );

        let mut binds = vec![
            format!("{}:{}", data_dir, mount_point),
            format!("{}:/archive", archive_dir),
            format!("{}:/backups", backup_dir),
        ];

        if let Some(rd) = restore_dir {
            binds.push(format!("{}:/restore", rd));
        }


        let config = Config {
            image: Some(image),
            user: Some(user_str),
            env: Some(vec![
                "POSTGRES_PASSWORD=password".to_string(),
                "POSTGRES_HOST_AUTH_METHOD=trust".to_string(),
            ]),
            host_config: Some(HostConfig {
                port_bindings: Some(port_bindings),
                binds: Some(binds),
                ..Default::default()
            }),
            cmd: Some(vec![
                "postgres".to_string(),
                "-c".to_string(),
                "wal_level=replica".to_string(),
                "-c".to_string(),
                "archive_mode=on".to_string(),
                "-c".to_string(),
                "archive_command=cp %p /archive/%f".to_string(),
            ]),
            ..Default::default()
        };



        let options = CreateContainerOptions {
            name: format!("paagan-{}", name),
            ..Default::default()
        };

        let response = self.docker.create_container(Some(options), config).await?;
        self.docker.start_container(&response.id, None::<StartContainerOptions<String>>).await?;

        Ok(response.id)
    }

    pub async fn list_instances(&self) -> Result<Vec<(String, String)>> {
        let mut filters = HashMap::new();
        filters.insert("name".to_string(), vec!["paagan-".to_string()]);
        
        let options = ListContainersOptions {
            all: true,
            filters,
            ..Default::default()
        };

        let containers = self.docker.list_containers(Some(options)).await?;
        let mut result = Vec::new();
        for container in containers {
            if let Some(names) = container.names {
                for name in names {
                    if let Some(instance_name) = name.strip_prefix("/paagan-") {
                        result.push((instance_name.to_string(), container.state.clone().unwrap_or_default()));
                    }
                }
            }
        }
        Ok(result)
    }

    pub async fn start_container(&self, name: &str) -> Result<()> {
        let container_name = format!("paagan-{}", name);
        self.docker.start_container(&container_name, None::<StartContainerOptions<String>>).await?;
        Ok(())
    }

    pub async fn stop_container(&self, name: &str) -> Result<()> {
        let container_name = format!("paagan-{}", name);
        self.docker.stop_container(&container_name, None).await?;
        Ok(())
    }

    pub async fn delete_container(&self, name: &str) -> Result<()> {
        let container_name = format!("paagan-{}", name);
        let options = bollard::container::RemoveContainerOptions {
            force: true,
            ..Default::default()
        };
        self.docker.remove_container(&container_name, Some(options)).await?;
        Ok(())
    }

    pub async fn run_basebackup(&self, name: &str, backup_path: &str) -> Result<()> {
        let container_name = format!("paagan-{}", name);
        let status = std::process::Command::new("docker")
            .arg("exec")
            .arg(container_name)
            .arg("pg_basebackup")
            .arg("-U")
            .arg("postgres")
            .arg("-D")
            .arg(backup_path)
            .arg("--checkpoint=fast")
            .arg("-Ft")
            .arg("-z")
            .arg("-X")
            .arg("stream")
            .arg("-P")
            .status()
            .context("Failed to run pg_basebackup")?;
        
        if !status.success() {
            anyhow::bail!("pg_basebackup failed");
        }
        Ok(())
    }

    pub async fn run_wal_switch(&self, name: &str) -> Result<()> {
        let container_name = format!("paagan-{}", name);
        let status = std::process::Command::new("docker")
            .arg("exec")
            .arg(container_name)
            .arg("psql")
            .arg("-U")
            .arg("postgres")
            .arg("-c")
            .arg("SELECT pg_switch_wal();")
            .status()
            .context("Failed to run pg_switch_wal")?;
        
        if !status.success() {
            anyhow::bail!("pg_switch_wal failed");
        }
        Ok(())
    }

    pub async fn exec_psql(&self, name: &str) -> Result<()> {
        let container_name = format!("paagan-{}", name);
        // bollard exec is more complex for interactive, so we'll use std::process::Command for psql
        let status = std::process::Command::new("docker")
            .arg("exec")
            .arg("-it")
            .arg(container_name)
            .arg("psql")
            .arg("-U")
            .arg("postgres")
            .status()
            .context("Failed to execute docker exec")?;
        
        if !status.success() {
            anyhow::bail!("psql command failed");
        }
        Ok(())
    }
}
