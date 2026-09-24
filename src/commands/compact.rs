use crate::CommandOutput;
use crate::backups::{self, human_bytes};
use crate::commands::Outputs;
use crate::config::ConfigManager;
use crate::docker::DockerManager;
use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use serde::Serialize;
use std::fs;
use std::io::ErrorKind;

#[derive(Serialize)]
pub struct CompactResult {
    pub name: String,
    pub dry_run: bool,
    pub backups_kept: Vec<String>,
    pub backups_removed: Vec<String>,
    pub wal_files_removed: usize,
    pub bytes_freed: u64,
    /// Oldest time a `fork --at` can target after this compaction.
    pub restorable_from: String,
}

#[derive(Serialize)]
pub struct CompactOutput {
    pub results: Vec<CompactResult>,
    pub skipped: Vec<String>,
}

impl CommandOutput for CompactOutput {
    fn to_text(&self) -> String {
        let mut lines = Vec::new();
        for r in &self.results {
            let verb = if r.dry_run { "Would free" } else { "Freed" };
            lines.push(format!(
                "{}: {} {} ({} WAL files, {} backups removed; {} kept)\n  PITR window: {} -> now",
                r.name,
                verb,
                human_bytes(r.bytes_freed),
                r.wal_files_removed,
                r.backups_removed.len(),
                r.backups_kept.len(),
                r.restorable_from,
            ));
        }
        for s in &self.skipped {
            lines.push(format!("Skipped {}", s));
        }
        lines.join("\n")
    }
}

pub async fn compact(
    config_mgr: &ConfigManager,
    docker_mgr: &DockerManager,
    name: Option<String>,
    all: bool,
    retain: String,
    strict: bool,
    dry_run: bool,
) -> Result<Outputs> {
    let retain = backups::parse_duration(&retain)?;
    let mut results = Vec::new();
    let mut skipped = Vec::new();

    if all {
        let mut names: Vec<String> = config_mgr.load_config()?.instances.into_keys().collect();
        names.sort();
        for name in names {
            if !docker_mgr.is_running(&name).await? {
                skipped.push(format!("{}: not running", name));
                continue;
            }
            match compact_instance(config_mgr, docker_mgr, &name, retain, strict, dry_run).await {
                Ok(r) => results.push(r),
                Err(e) => skipped.push(format!("{}: {:#}", name, e)),
            }
        }
    } else {
        let name = name.context("Specify an instance name or --all")?;
        results
            .push(compact_instance(config_mgr, docker_mgr, &name, retain, strict, dry_run).await?);
    }

    Ok(Outputs::Compact(CompactOutput { results, skipped }))
}

async fn compact_instance(
    config_mgr: &ConfigManager,
    docker_mgr: &DockerManager,
    name: &str,
    retain: Duration,
    strict: bool,
    dry_run: bool,
) -> Result<CompactResult> {
    let metadata = config_mgr.get_instance(name)?;
    if !docker_mgr.is_running(name).await? {
        anyhow::bail!(
            "Instance '{}' is not running; start it before compacting",
            name
        );
    }
    let instance_dir = config_mgr.get_instance_dir(name);
    let seg_size = docker_mgr
        .wal_segment_size(name, metadata.init_mode)
        .await?;
    let cutoff = Utc::now() - retain;

    // A fresh backup always survives, so it is the newest PITR anchor. In a
    // dry run we don't take it; the current WAL segment stands in for where
    // it would start.
    let (new_backup_dir, pending_start_segment) = if dry_run {
        let current = docker_mgr
            .query_scalar(
                name,
                metadata.init_mode,
                "SELECT pg_walfile_name(pg_current_wal_lsn())",
            )
            .await?;
        (None, Some(current))
    } else {
        eprintln!(
            "[{}] Compacting '{}': taking base backup...",
            Utc::now().to_rfc3339(),
            name
        );
        let dir =
            backups::take_base_backup(docker_mgr, name, &instance_dir, metadata.init_mode).await?;
        docker_mgr.run_wal_switch(name, metadata.init_mode).await?;
        (Some(dir), None)
    };

    let (kept, removed) = backups::select_retained(
        backups::list_backups(&instance_dir)?,
        cutoff,
        new_backup_dir.as_deref(),
        strict,
    );

    let threshold = kept
        .iter()
        .map(|b| b.start_segment(seg_size))
        .chain(pending_start_segment)
        .min_by(|a, b| a[8..].cmp(&b[8..]))
        .context("No base backup to anchor the archive")?;

    let wal_files = backups::archive_files_before(&instance_dir.join("archive"), &threshold)?;
    let mut bytes_freed: u64 = wal_files.iter().map(|(_, size)| size).sum();
    for b in &removed {
        bytes_freed += backups::dir_size(&b.dir)?;
    }

    if !dry_run {
        eprintln!(
            "Removing {} WAL files before {} and {} old backups...",
            wal_files.len(),
            threshold,
            removed.len()
        );
        // NotFound is fine: an overlapping compaction may have removed it.
        for (path, _) in &wal_files {
            if let Err(e) = fs::remove_file(path)
                && e.kind() != ErrorKind::NotFound
            {
                return Err(e).with_context(|| format!("Failed to remove {}", path.display()));
            }
        }
        for b in &removed {
            if let Err(e) = fs::remove_dir_all(&b.dir)
                && e.kind() != ErrorKind::NotFound
            {
                return Err(e).with_context(|| format!("Failed to remove {}", b.dir.display()));
            }
        }
    }

    let restorable_from = kept
        .first()
        .map(|b| b.taken_at)
        .unwrap_or_else(Utc::now)
        .to_rfc3339();

    Ok(CompactResult {
        name: name.to_string(),
        dry_run,
        backups_kept: kept.into_iter().map(|b| b.name).collect(),
        backups_removed: removed.into_iter().map(|b| b.name).collect(),
        wal_files_removed: wal_files.len(),
        bytes_freed,
        restorable_from,
    })
}
