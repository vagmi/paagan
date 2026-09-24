use crate::config::InitMode;
use crate::docker::DockerManager;
use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use std::fs;
use std::path::{Path, PathBuf};

const BACKUP_DIR_FORMAT: &str = "%Y%m%dT%H%M%SZ";

#[derive(Debug, Clone)]
pub struct BaseBackup {
    pub name: String,
    pub dir: PathBuf,
    /// When the backup completed (mtime of `backup_manifest`, which
    /// pg_basebackup writes last). Recovery targets must be at or after this.
    pub taken_at: DateTime<Utc>,
    pub timeline: u32,
    pub start_lsn: u64,
}

impl BaseBackup {
    pub fn start_segment(&self, wal_segment_size: u64) -> String {
        segment_name(self.timeline, self.start_lsn, wal_segment_size)
    }
}

/// Path inside the container (`/backups` is the instance's backups dir) for a
/// new base backup taken at `now`.
pub fn new_backup_path(now: DateTime<Utc>) -> String {
    format!("/backups/{}", now.format(BACKUP_DIR_FORMAT))
}

/// Host path of the backup dir created by `new_backup_path`.
pub fn host_backup_dir(instance_dir: &Path, container_path: &str) -> PathBuf {
    let name = container_path.trim_start_matches("/backups/");
    instance_dir.join("backups").join(name)
}

/// Take a new base backup into a timestamped dir under `backups/`. A failed
/// pg_basebackup's partial output is removed so it never looks like a backup.
pub async fn take_base_backup(
    docker_mgr: &DockerManager,
    name: &str,
    instance_dir: &Path,
    init_mode: InitMode,
) -> Result<PathBuf> {
    let container_path = new_backup_path(Utc::now());
    let host_dir = host_backup_dir(instance_dir, &container_path);
    if let Err(e) = docker_mgr
        .run_basebackup(name, &container_path, init_mode)
        .await
    {
        if host_dir.exists() {
            let _ = fs::remove_dir_all(&host_dir);
        }
        return Err(e);
    }
    Ok(host_dir)
}

/// All completed base backups of an instance, oldest first. Directories
/// without a readable `backup_manifest` (e.g. an interrupted pg_basebackup)
/// are skipped.
pub fn list_backups(instance_dir: &Path) -> Result<Vec<BaseBackup>> {
    let backups_dir = instance_dir.join("backups");
    let mut backups = Vec::new();
    if !backups_dir.exists() {
        return Ok(backups);
    }

    for entry in fs::read_dir(&backups_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let dir = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let manifest_path = dir.join("backup_manifest");
        let Ok(manifest) = fs::read_to_string(&manifest_path) else {
            continue;
        };
        let (timeline, start_lsn) = parse_manifest_start(&manifest)
            .with_context(|| format!("Invalid backup manifest in {}", dir.display()))?;
        let taken_at: DateTime<Utc> = fs::metadata(&manifest_path)?.modified()?.into();
        backups.push(BaseBackup {
            name,
            dir,
            taken_at,
            timeline,
            start_lsn,
        });
    }

    backups.sort_by_key(|b| b.taken_at);
    Ok(backups)
}

/// Split backups (oldest first) into (kept, removed) for a retention cutoff.
/// Backups newer than the cutoff and `always_keep` are kept. Unless `strict`,
/// the newest backup older than the cutoff is kept too: it anchors the start
/// of the window, so PITR reaches back the full retention period.
pub fn select_retained(
    backups: Vec<BaseBackup>,
    cutoff: DateTime<Utc>,
    always_keep: Option<&Path>,
    strict: bool,
) -> (Vec<BaseBackup>, Vec<BaseBackup>) {
    let anchor = if strict {
        None
    } else {
        backups
            .iter()
            .rev()
            .find(|b| b.taken_at < cutoff)
            .map(|b| b.dir.clone())
    };
    backups.into_iter().partition(|b| {
        b.taken_at >= cutoff
            || Some(b.dir.as_path()) == always_keep
            || Some(&b.dir) == anchor.as_ref()
    })
}

/// The newest backup that completed at or before `target`.
pub fn backup_for_target(backups: &[BaseBackup], target: DateTime<Utc>) -> Option<&BaseBackup> {
    backups.iter().rev().find(|b| b.taken_at <= target)
}

/// Parse a recovery target as postgres would print it (`now()` output) or as
/// RFC 3339. Values without an offset are treated as UTC, which is the
/// timezone of the postgres containers paagan runs.
pub fn parse_target_time(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Some(t.with_timezone(&Utc));
    }
    for fmt in ["%Y-%m-%d %H:%M:%S%.f%#z", "%Y-%m-%dT%H:%M:%S%.f%#z"] {
        if let Ok(t) = DateTime::parse_from_str(s, fmt) {
            return Some(t.with_timezone(&Utc));
        }
    }
    for fmt in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.f"] {
        if let Ok(t) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(t.and_utc());
        }
    }
    None
}

/// Parse a retention like `7d`, `24h` or `30m`.
pub fn parse_duration(s: &str) -> Result<chrono::Duration> {
    let s = s.trim();
    let (num, unit) = s.split_at(s.len().saturating_sub(1));
    let n: i64 = num
        .parse()
        .with_context(|| format!("Invalid duration '{}', expected e.g. 7d, 24h, 30m", s))?;
    match unit {
        "d" => Ok(chrono::Duration::days(n)),
        "h" => Ok(chrono::Duration::hours(n)),
        "m" => Ok(chrono::Duration::minutes(n)),
        _ => anyhow::bail!("Invalid duration '{}', expected e.g. 7d, 24h, 30m", s),
    }
}

/// Returns (timeline, start LSN) from the first `WAL-Ranges` entry.
fn parse_manifest_start(manifest: &str) -> Result<(u32, u64)> {
    let json: serde_json::Value = serde_json::from_str(manifest)?;
    let range = json
        .get("WAL-Ranges")
        .and_then(|r| r.get(0))
        .context("Missing WAL-Ranges")?;
    let timeline = range
        .get("Timeline")
        .and_then(|t| t.as_u64())
        .context("Missing Timeline")? as u32;
    let lsn = range
        .get("Start-LSN")
        .and_then(|l| l.as_str())
        .context("Missing Start-LSN")?;
    Ok((timeline, parse_lsn(lsn)?))
}

fn parse_lsn(s: &str) -> Result<u64> {
    let (hi, lo) = s.split_once('/').context("LSN must look like X/Y")?;
    Ok((u64::from_str_radix(hi, 16)? << 32) | u64::from_str_radix(lo, 16)?)
}

fn segment_name(timeline: u32, lsn: u64, wal_segment_size: u64) -> String {
    let segs_per_id = 0x1_0000_0000 / wal_segment_size;
    let segno = lsn / wal_segment_size;
    format!(
        "{:08X}{:08X}{:08X}",
        timeline,
        segno / segs_per_id,
        segno % segs_per_id
    )
}

/// Archive files that are no longer needed once the oldest kept base backup
/// starts at `threshold`. Mirrors pg_archivecleanup: WAL segments, `.partial`
/// and `.backup` files whose log/segment part (ignoring the timeline) sorts
/// before the threshold's. Timeline `.history` files are always kept.
pub fn archive_files_before(archive_dir: &Path, threshold: &str) -> Result<Vec<(PathBuf, u64)>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(archive_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if is_removable_before(&name, threshold) {
            files.push((entry.path(), entry.metadata()?.len()));
        }
    }
    files.sort();
    Ok(files)
}

fn is_removable_before(name: &str, threshold: &str) -> bool {
    if name.len() < 24 || !name[..24].bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    let rest = &name[24..];
    let is_wal = rest.is_empty() || rest == ".partial" || rest.ends_with(".backup");
    is_wal && name[8..24] < threshold[8..24]
}

pub fn dir_size(path: &Path) -> Result<u64> {
    let mut total = 0;
    if !path.exists() {
        return Ok(0);
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        total += if meta.is_dir() {
            dir_size(&entry.path())?
        } else {
            meta.len()
        };
    }
    Ok(total)
}

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} B", bytes)
    } else {
        format!("{:.1} {}", value, UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    const SEG_16MB: u64 = 16 * 1024 * 1024;

    fn backup(name: &str, taken_at: DateTime<Utc>) -> BaseBackup {
        BaseBackup {
            name: name.to_string(),
            dir: PathBuf::from(name),
            taken_at,
            timeline: 1,
            start_lsn: 0,
        }
    }

    #[test]
    fn lsn_to_segment_name() {
        assert_eq!(
            segment_name(1, parse_lsn("0/2000028").unwrap(), SEG_16MB),
            "000000010000000000000002"
        );
        assert_eq!(
            segment_name(1, parse_lsn("0/FF000000").unwrap(), SEG_16MB),
            "0000000100000000000000FF"
        );
        assert_eq!(
            segment_name(2, parse_lsn("21/23000000").unwrap(), SEG_16MB),
            "000000020000002100000023"
        );
        assert_eq!(
            segment_name(1, parse_lsn("1/0").unwrap(), SEG_16MB),
            "000000010000000100000000"
        );
        // 64MB segments: 64 segments per xlogid
        assert_eq!(
            segment_name(1, parse_lsn("1/C000000").unwrap(), 64 * 1024 * 1024),
            "000000010000000100000003"
        );
    }

    #[test]
    fn manifest_start() {
        let manifest = r#"{"PostgreSQL-Backup-Manifest-Version": 1,
            "WAL-Ranges": [{"Timeline": 1, "Start-LSN": "0/2000028", "End-LSN": "0/2000120"}]}"#;
        assert_eq!(parse_manifest_start(manifest).unwrap(), (1, 0x2000028));
    }

    #[test]
    fn archive_cleanup_rule() {
        let threshold = "000000010000002100000010";
        assert!(is_removable_before("00000001000000210000000F", threshold));
        assert!(is_removable_before("000000010000000000000002", threshold));
        assert!(is_removable_before(
            "000000010000000000000002.00000028.backup",
            threshold
        ));
        assert!(is_removable_before(
            "00000001000000210000000F.partial",
            threshold
        ));
        // timeline is ignored, like pg_archivecleanup
        assert!(is_removable_before("00000002000000210000000F", threshold));

        assert!(!is_removable_before("000000010000002100000010", threshold));
        assert!(!is_removable_before("000000010000002100000011", threshold));
        assert!(!is_removable_before("00000002.history", threshold));
        assert!(!is_removable_before(".DS_Store", threshold));
        assert!(!is_removable_before(
            "00000001000000210000000F.tmp",
            threshold
        ));
    }

    #[test]
    fn picks_newest_backup_before_target() {
        let t = |h| Utc.with_ymd_and_hms(2026, 9, 1, h, 0, 0).unwrap();
        let backups = vec![backup("a", t(1)), backup("b", t(5)), backup("c", t(9))];
        assert_eq!(backup_for_target(&backups, t(6)).unwrap().name, "b");
        assert_eq!(backup_for_target(&backups, t(5)).unwrap().name, "b");
        assert_eq!(backup_for_target(&backups, t(10)).unwrap().name, "c");
        assert!(backup_for_target(&backups, t(0)).is_none());
    }

    #[test]
    fn retention_keeps_anchor_unless_strict() {
        let d = |day| Utc.with_ymd_and_hms(2026, 9, day, 3, 0, 0).unwrap();
        let backups = || {
            vec![
                backup("d1", d(1)),
                backup("d10", d(10)),
                backup("d14", d(14)),
                backup("d20", d(20)),
            ]
        };
        let names = |v: &[BaseBackup]| v.iter().map(|b| b.name.clone()).collect::<Vec<_>>();
        let cutoff = d(13);

        let (kept, removed) = select_retained(backups(), cutoff, None, false);
        assert_eq!(names(&kept), ["d10", "d14", "d20"]);
        assert_eq!(names(&removed), ["d1"]);

        let (kept, removed) = select_retained(backups(), cutoff, None, true);
        assert_eq!(names(&kept), ["d14", "d20"]);
        assert_eq!(names(&removed), ["d1", "d10"]);

        // everything older than the cutoff: only the anchor and the pinned backup survive
        let (kept, _) = select_retained(backups(), d(25), Some(Path::new("d1")), false);
        assert_eq!(names(&kept), ["d1", "d20"]);
        let (kept, _) = select_retained(backups(), d(25), Some(Path::new("d20")), true);
        assert_eq!(names(&kept), ["d20"]);
    }

    #[test]
    fn target_time_formats() {
        let expected = Utc.with_ymd_and_hms(2026, 9, 23, 17, 15, 11).unwrap();
        for s in [
            "2026-09-23 17:15:11+00",
            "2026-09-23 17:15:11+00:00",
            "2026-09-23 22:45:11+05:30",
            "2026-09-23T17:15:11Z",
            "2026-09-23 17:15:11",
        ] {
            assert_eq!(parse_target_time(s), Some(expected), "{}", s);
        }
        assert_eq!(
            parse_target_time("2026-09-23 17:15:11.5+00"),
            Some(expected + chrono::Duration::milliseconds(500))
        );
        assert_eq!(parse_target_time("yesterday"), None);
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("7d").unwrap(), chrono::Duration::days(7));
        assert_eq!(parse_duration("24h").unwrap(), chrono::Duration::hours(24));
        assert_eq!(parse_duration("0h").unwrap(), chrono::Duration::zero());
        assert!(parse_duration("7").is_err());
        assert!(parse_duration("d").is_err());
        assert!(parse_duration("7w").is_err());
    }

    #[test]
    fn backup_path_roundtrip() {
        let now = Utc.with_ymd_and_hms(2026, 9, 23, 17, 15, 11).unwrap();
        let path = new_backup_path(now);
        assert_eq!(path, "/backups/20260923T171511Z");
        assert_eq!(
            host_backup_dir(Path::new("/i"), &path),
            PathBuf::from("/i/backups/20260923T171511Z")
        );
    }
}
