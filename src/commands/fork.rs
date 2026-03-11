use crate::commands::Outputs;
use crate::config::{ConfigManager, InstanceMetadata};
use crate::docker::DockerManager;
use crate::CommandOutput;
use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use portpicker::pick_unused_port;
use serde::Serialize;
use std::fs;
use std::io::Write;
use tar::Archive;

#[derive(Serialize)]
pub struct ForkOutput {
    pub old_name: String,
    pub new_name: String,
    pub port: u16,
    pub connection_string: String,
}

impl CommandOutput for ForkOutput {
    fn to_text(&self) -> String {
        format!(
            "Forked '{}' to '{}' on port {}\nConnection string: {}",
            self.old_name, self.new_name, self.port, self.connection_string
        )
    }
}

pub async fn fork_instance(
    config_mgr: &ConfigManager,
    docker_mgr: &DockerManager,
    at: Option<String>,
    old_name: String,
    new_name: String,
) -> Result<Outputs> {
    let old_metadata = config_mgr.get_instance(&old_name)?;
    let port = pick_unused_port().context("No available ports")?;

    // 1. Prepare new instance dirs
    config_mgr.create_instance_dirs(&new_name).await?;
    let old_dir = config_mgr.get_instance_dir(&old_name);
    let new_dir = config_mgr.get_instance_dir(&new_name);

    // 2. Base backup of source
    let local_backup_snapshot_dir = old_dir.join("backups").join("base_snapshot");
    let backup_snapshot_dir = "/backups/base_snapshot";

    if at.is_none() || !local_backup_snapshot_dir.exists() {
        eprintln!("Taking base backup of '{}'...", old_name);
        // Clean up old backup if exists
        if local_backup_snapshot_dir.exists() {
            fs::remove_dir_all(&local_backup_snapshot_dir)?;
        }

        docker_mgr
            .run_basebackup(&old_name, backup_snapshot_dir)
            .await?;
    } else {
        eprintln!("Using existing base backup of '{}' for PITR...", old_name);
    }

    eprintln!("Switching WAL on source to ensure all logs are archived...");
    docker_mgr.run_wal_switch(&old_name).await?;
    // Grace period to ensure background archiver finishes copying WAL files to the archive directory
    eprintln!("Waiting 10s for WAL archiving to complete...");
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;

    // 3. Extract backup to new data dir
    eprintln!("Extracting backup to '{}'...", new_name);
    let base_tar = local_backup_snapshot_dir.join("base.tar");
    let base_tar_gz = local_backup_snapshot_dir.join("base.tar.gz");

    let (_, new_data_subdir) = docker_mgr.get_mount_info(&old_metadata.version);
    let new_data_root = new_dir.join("data");
    let new_actual_data_dir = if let Some(ref sub) = new_data_subdir {
        new_data_root.join(sub)
    } else {
        new_data_root.clone()
    };

    // Clean new data dir
    if new_data_root.exists() {
        fs::remove_dir_all(&new_data_root)?;
    }
    fs::create_dir_all(&new_actual_data_dir)?;

    if base_tar_gz.exists() {
        let tar_gz = fs::File::open(base_tar_gz)?;
        let tar = GzDecoder::new(tar_gz);
        let mut archive = Archive::new(tar);
        archive.unpack(&new_actual_data_dir)?;
    } else if base_tar.exists() {
        let tar_file = fs::File::open(base_tar)?;
        let mut archive = Archive::new(tar_file);
        archive.unpack(&new_actual_data_dir)?;
    } else {
        anyhow::bail!("Base backup file not found");
    }

    // 4. Prepare recovery
    // Remove old WALs
    let new_wal_dir = new_actual_data_dir.join("pg_wal");
    if new_wal_dir.exists() {
        fs::remove_dir_all(&new_wal_dir)?;
        fs::create_dir_all(&new_wal_dir)?;
    }

    // Create recovery.signal
    fs::File::create(new_actual_data_dir.join("recovery.signal"))?;

    // Configure recovery
    let mut postgresql_conf = fs::OpenOptions::new()
        .append(true)
        .open(new_actual_data_dir.join("postgresql.conf"))?;

    writeln!(postgresql_conf, "\n# Paagan Recovery Config")?;
    writeln!(postgresql_conf, "restore_command = 'cp /restore/%f %p'")?;
    if let Some(target_time) = at {
        writeln!(postgresql_conf, "recovery_target_time = '{}'", target_time)?;
    }
    writeln!(postgresql_conf, "recovery_target_action = 'promote'")?;

    // 5. Start new container
    eprintln!("Starting new instance '{}'...", new_name);
    let new_archive_dir = new_dir.join("archive").to_string_lossy().to_string();
    let new_backup_dir = new_dir.join("backups").to_string_lossy().to_string();
    let old_archive_dir = old_dir.join("archive").to_string_lossy().to_string();

    let container_id = docker_mgr
        .create_instance_container(
            &new_name,
            &old_metadata.version,
            port,
            &new_data_root.to_string_lossy().to_string(),
            &new_archive_dir,
            &new_backup_dir,
            Some(&old_archive_dir),
        )
        .await?;

    let metadata = InstanceMetadata {
        name: new_name.clone(),
        version: old_metadata.version,
        port,
        container_id: Some(container_id),
        data_subdir: new_data_subdir,
    };
    config_mgr.add_instance(metadata)?;

    let connection_string = format!("postgresql://postgres@localhost:{}/postgres", port);
    eprintln!("Forked '{}' to '{}' on port {}", old_name, new_name, port);

    Ok(Outputs::Fork(ForkOutput {
        old_name,
        new_name,
        port,
        connection_string,
    }))
}
