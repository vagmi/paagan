use crate::CommandOutput;
use crate::backups::{self, human_bytes};
use crate::commands::Outputs;
use crate::commands::schedule::{self, ScheduleStatus};
use crate::config::{ConfigManager, InitMode};
use anyhow::Result;
use serde::Serialize;

#[derive(Serialize)]
pub struct ShowOutput {
    pub name: String,
    pub version: String,
    pub port: u16,
    pub password: String,
    pub connection_string: String,
    pub data_directory: String,
    pub image: String,
    pub init_mode: String,
    pub shared_preload_libraries: Option<String>,
    pub archive_bytes: u64,
    /// Oldest time `fork --at` can target, if any base backup exists.
    pub restorable_from: Option<String>,
    pub compact_schedules: Vec<ScheduleStatus>,
}

impl CommandOutput for ShowOutput {
    fn to_text(&self) -> String {
        let mut out = format!(
            "Instance: {}\nVersion: {}\nImage: {}\nInit mode: {}\nPort: {}\nPassword: {}\nConnection string: {}\nData directory: {}",
            self.name,
            self.version,
            self.image,
            self.init_mode,
            self.port,
            self.password,
            self.connection_string,
            self.data_directory
        );
        if let Some(spls) = &self.shared_preload_libraries {
            out.push_str(&format!("\nshared_preload_libraries: {}", spls));
        }
        out.push_str(&format!(
            "\nWAL archive: {}",
            human_bytes(self.archive_bytes)
        ));
        match &self.restorable_from {
            Some(t) => out.push_str(&format!("\nPITR window: {} -> now", t)),
            None => out.push_str("\nPITR window: none (no base backup)"),
        }
        if self.compact_schedules.is_empty() {
            out.push_str("\nCompaction schedule: none");
        }
        for s in &self.compact_schedules {
            out.push_str(&format!(
                "\nCompaction schedule: '{}' retain {}{}{} [{}]",
                s.cron,
                s.retain,
                if s.strict { " (strict)" } else { "" },
                if s.all_instances { " via --all" } else { "" },
                s.state
            ));
        }
        out
    }
}

pub async fn show_instance(config_mgr: &ConfigManager, name: String) -> Result<Outputs> {
    let metadata = config_mgr.get_instance(&name)?;
    let connection_string = metadata.connection_string();
    let password = metadata.default_password().to_string();
    let instance_dir = config_mgr.get_instance_dir(&name);
    let archive_bytes = backups::dir_size(&instance_dir.join("archive"))?;
    let restorable_from = backups::list_backups(&instance_dir)?
        .first()
        .map(|b| b.taken_at.to_rfc3339());
    let compact_schedules = schedule::schedules_for(config_mgr, &name)?;
    let data_directory = instance_dir.join("data").to_string_lossy().to_string();
    let image = metadata.resolved_image();
    let init_mode = match metadata.init_mode {
        InitMode::Standard => "standard",
        InitMode::Cnpg => "cnpg",
    }
    .to_string();

    Ok(Outputs::Show(ShowOutput {
        name: metadata.name,
        version: metadata.version,
        port: metadata.port,
        password,
        connection_string,
        data_directory,
        image,
        init_mode,
        shared_preload_libraries: metadata.shared_preload_libraries,
        archive_bytes,
        restorable_from,
        compact_schedules,
    }))
}
