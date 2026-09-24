use crate::CommandOutput;
use crate::backups;
use crate::commands::Outputs;
use crate::config::{CompactSchedule, ConfigManager};
use anyhow::{Context, Result};
use native_cron::CronOptions;
use serde::Serialize;
use std::path::PathBuf;

/// Job id for `--all` uses `_` so it can never collide with an instance's
/// `paagan-compact-<name>` id.
const ALL_JOB_ID: &str = "paagan_compact_all";

#[derive(Serialize)]
pub struct ScheduleOutput {
    pub job_id: String,
    pub instance: Option<String>,
    pub scheduled: bool,
    pub cron: Option<String>,
    pub command: Option<Vec<String>>,
    pub log: Option<String>,
}

impl CommandOutput for ScheduleOutput {
    fn to_text(&self) -> String {
        let target = match &self.instance {
            Some(name) => format!("'{}'", name),
            None => "all instances".to_string(),
        };
        match (&self.cron, &self.log) {
            (Some(cron), Some(log)) if self.scheduled => format!(
                "Scheduled compaction of {} at '{}' ({})\nLog: {}",
                target, cron, self.job_id, log
            ),
            _ => format!("Removed compaction schedule for {}", target),
        }
    }
}

pub fn job_id(instance: Option<&str>) -> Result<String> {
    let Some(name) = instance else {
        return Ok(ALL_JOB_ID.to_string());
    };
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        anyhow::bail!(
            "Can't schedule '{}': the OS scheduler only allows letters, numbers, '-' and '_' in job names",
            name
        );
    }
    Ok(format!("paagan-compact-{}", name))
}

pub fn schedule_compact(
    config_mgr: &ConfigManager,
    instance: Option<String>,
    cron: String,
    retain: String,
    strict: bool,
) -> Result<Outputs> {
    backups::parse_duration(&retain)?;
    if let Some(name) = &instance {
        config_mgr.get_instance(name)?;
    }
    let job_id = job_id(instance.as_deref())?;

    let exe = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .context("Could not locate the paagan executable")?;
    let mut command = vec![
        exe.to_string_lossy().to_string(),
        "compact".to_string(),
        "--retain".to_string(),
        retain.clone(),
    ];
    if strict {
        command.push("--strict".to_string());
    }
    command.push(instance.clone().unwrap_or_else(|| "--all".to_string()));

    let log: PathBuf = config_mgr.logs_dir().join(format!("{}.log", job_id));

    // Schedulers start jobs with a minimal environment; carry over what we need
    // to find and talk to docker.
    let mut options = CronOptions::new(job_id.clone(), cron.clone(), command.clone())
        .stdout(&log)
        .stderr(&log)
        .overwrite(true);
    for var in [
        "PATH",
        "HOME",
        "DOCKER_HOST",
        "DOCKER_CONTEXT",
        "DOCKER_CONFIG",
    ] {
        if let Ok(value) = std::env::var(var) {
            options = options.env(var, value);
        }
    }
    native_cron::register(options)
        .with_context(|| format!("Failed to register schedule '{}'", cron))?;

    let mut config = config_mgr.load_config()?;
    config.compact_schedules.insert(
        job_id.clone(),
        CompactSchedule {
            instance: instance.clone(),
            cron: cron.clone(),
            retain,
            strict,
        },
    );
    config_mgr.save_config(&config)?;

    Ok(Outputs::Schedule(ScheduleOutput {
        job_id,
        instance,
        scheduled: true,
        cron: Some(cron),
        command: Some(command),
        log: Some(log.to_string_lossy().to_string()),
    }))
}

pub fn unschedule_compact(config_mgr: &ConfigManager, instance: Option<String>) -> Result<Outputs> {
    let job_id = job_id(instance.as_deref())?;
    native_cron::remove(&job_id)
        .with_context(|| format!("Failed to remove scheduled job '{}'", job_id))?;

    let mut config = config_mgr.load_config()?;
    if config.compact_schedules.remove(&job_id).is_some() {
        config_mgr.save_config(&config)?;
    }

    Ok(Outputs::Schedule(ScheduleOutput {
        job_id,
        instance,
        scheduled: false,
        cron: None,
        command: None,
        log: None,
    }))
}

/// Remove an instance's own schedule, if it has one. Used by `delete`.
pub fn remove_instance_schedule(config_mgr: &ConfigManager, name: &str) -> Result<()> {
    let Ok(job_id) = job_id(Some(name)) else {
        return Ok(());
    };
    if config_mgr
        .load_config()?
        .compact_schedules
        .contains_key(&job_id)
    {
        unschedule_compact(config_mgr, Some(name.to_string()))?;
    }
    Ok(())
}

#[derive(Serialize)]
pub struct ScheduleStatus {
    pub job_id: String,
    pub all_instances: bool,
    pub cron: String,
    pub retain: String,
    pub strict: bool,
    /// active / inactive / missing, as reported by the OS scheduler.
    pub state: String,
}

/// Schedules that compact `name`: its own and the `--all` one.
pub fn schedules_for(config_mgr: &ConfigManager, name: &str) -> Result<Vec<ScheduleStatus>> {
    let config = config_mgr.load_config()?;
    let mut result = Vec::new();
    for (job_id, s) in config.compact_schedules {
        if s.instance.as_deref().is_some_and(|i| i != name) {
            continue;
        }
        let state = native_cron::job(job_id.clone())
            .and_then(|job| job.status())
            .map(|status| format!("{:?}", status.state).to_lowercase())
            .unwrap_or_else(|e| format!("unknown ({})", e));
        result.push(ScheduleStatus {
            job_id,
            all_instances: s.instance.is_none(),
            cron: s.cron,
            retain: s.retain,
            strict: s.strict,
            state,
        });
    }
    result.sort_by(|a, b| a.job_id.cmp(&b.job_id));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_ids() {
        assert_eq!(job_id(None).unwrap(), "paagan_compact_all");
        assert_eq!(job_id(Some("chatdb")).unwrap(), "paagan-compact-chatdb");
        assert_eq!(
            job_id(Some("slop_store")).unwrap(),
            "paagan-compact-slop_store"
        );
        assert_ne!(job_id(Some("all")).unwrap(), job_id(None).unwrap());
        assert!(job_id(Some("my.db")).is_err());
    }
}
