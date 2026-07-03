use chrono::{Datelike, Local, NaiveDateTime, TimeZone, Timelike};
use regex::Regex;
use std::convert::TryFrom;
use std::path;

use crate::config::types::RetentionConfig;
use crate::error::Result;
use crate::inject::{BackmaticContext, CommandRequest};

pub fn rsync_retention(
    ctx: &BackmaticContext,
    dest: &str,
    retention: &RetentionConfig,
) -> Result<()> {
    let re = Regex::new(r"^(([a-zA-Z][a-zA-Z_-]*)@)?([a-zA-Z][.a-zA-Z0-9_-]*):").unwrap();
    if re.is_match(dest) {
        log::info!("Skipping rsync retention for remote dest {dest}");
        return Ok(());
    }
    retain_for_period(ctx, dest, "hourly", 3600, retention.keep_hourly);
    retain_for_period(ctx, dest, "daily", 3600 * 24, retention.keep_daily);
    retain_for_period(ctx, dest, "weekly", 3600 * 24 * 7, retention.keep_weekly);
    retain_for_period(ctx, dest, "monthly", 3600 * 24 * 30, retention.keep_monthly);
    retain_for_period(ctx, dest, "yearly", 3600 * 24 * 364, retention.keep_yearly);
    Ok(())
}

fn retain_for_period(
    ctx: &BackmaticContext,
    dest: &str,
    retention: &str,
    interval: i64,
    count: i64,
) {
    if count < 1 {
        return;
    }
    if needs_retention(ctx, dest, retention, interval) {
        let target = format!(
            "{}.{}{}",
            dest.trim_end_matches('/'),
            retention,
            date_suffix(ctx)
        );
        let cmd = CommandRequest::new(ctx.paths.cp.to_string_lossy().to_string())
            .arg("-al")
            .arg(dest.trim_end_matches('/'))
            .arg(target);
        let _ = ctx.commands.run(&cmd);
    }
    prune_for_period(ctx, dest, retention, count);
}

fn needs_retention(ctx: &BackmaticContext, dest: &str, retention: &str, interval: i64) -> bool {
    let dest = dest.trim_end_matches('/');
    let parent = match path::Path::new(dest).parent() {
        Some(p) => p,
        None => return true,
    };
    let re = Regex::new(&format!("{}.{}", regex::escape(dest), retention)).unwrap();
    let Ok(entries) = parent.read_dir() else {
        return true;
    };
    for entry in entries.flatten() {
        if entry.path().is_dir() {
            if let Some(dir) = entry.path().to_str() {
                if dir.starts_with(&format!("{dest}.{retention}")) {
                    if let Ok(naive) =
                        NaiveDateTime::parse_from_str(&re.replace(dir, ""), "%Y-%m-%d-%H-%M")
                    {
                        if let Some(local) = Local.from_local_datetime(&naive).single() {
                            let period = ctx.clock.now().signed_duration_since(local);
                            if period.num_seconds() < interval {
                                return false;
                            }
                        }
                    }
                }
            }
        }
    }
    true
}

fn prune_for_period(ctx: &BackmaticContext, dest: &str, retention: &str, count: i64) {
    let dest = dest.trim_end_matches('/');
    let parent = match path::Path::new(dest).parent() {
        Some(p) => p,
        None => return,
    };
    let Ok(entries) = parent.read_dir() else {
        return;
    };
    let mut bup_list: Vec<String> = entries
        .filter_map(|r| r.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.path().to_str().map(String::from))
        .filter(|d| d.starts_with(&format!("{dest}.{retention}")))
        .collect();
    bup_list.sort();
    let size = usize::try_from(count).unwrap_or(0);
    if bup_list.len() > size {
        bup_list.truncate(bup_list.len() - size);
        for bupdir in bup_list {
            let cmd = CommandRequest::new(ctx.paths.rm.to_string_lossy().to_string())
                .arg("-rf")
                .arg(bupdir);
            let _ = ctx.commands.run(&cmd);
        }
    }
}

fn date_suffix(ctx: &BackmaticContext) -> String {
    let now = ctx.clock.now();
    format!(
        "{:02}-{:02}-{:02}-{:02}-{:02}",
        now.year(),
        now.month(),
        now.day(),
        now.hour(),
        now.minute(),
    )
}
