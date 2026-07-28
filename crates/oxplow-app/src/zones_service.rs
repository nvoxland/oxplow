//! Reading and writing the project's architectural zone table.
//!
//! Zones are project configuration, not oxplow's opinion (tsk251): the
//! `zones:` block in `.oxplow/project.yaml` is the whole vocabulary, and
//! an unconfigured project has none. This module is the seam the agent
//! drives through MCP (`list_zones` / `set_zones`) — read the table with
//! the distribution it actually produces, or replace it.
//!
//! Returning the distribution from BOTH calls is the point: a rule table
//! you can't see the effect of is guesswork. `unmatched_sample` is the
//! stale-table signal — paths the table doesn't cover yet, which is what
//! the repo growing a new top-level area looks like from here.

use std::collections::BTreeMap;

use oxplow_code_deps::{ZoneRules, ZONE_OTHER};
use oxplow_config::ZoneRuleConfig;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::Services;

/// How many unmatched paths to hand back. Enough to see the shape of
/// what's missing, few enough that the response stays readable.
const UNMATCHED_SAMPLE: usize = 20;

/// The zone table plus what it does to the current worktree.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ZoneReport {
    /// The project's rules, in evaluation order (first match wins).
    pub rules: Vec<ZoneRuleConfig>,
    /// File count per zone label, including `other`.
    pub distribution: BTreeMap<String, usize>,
    /// Files in the worktree the table classified.
    pub total_files: usize,
    /// Up to [`UNMATCHED_SAMPLE`] paths that fell through to `other` —
    /// the signal that the table needs a rule.
    pub unmatched_sample: Vec<String>,
}

/// Current rules + the distribution they produce over the worktree.
pub async fn zone_report(svc: &Services) -> Result<ZoneReport, String> {
    let rules = {
        let cfg = svc.config.read().unwrap_or_else(|e| e.into_inner());
        cfg.zones.clone()
    };
    report_for(svc, rules).await
}

/// Replace the table, persist it to `.oxplow/project.yaml`, and report
/// what the new rules match. Validation is the same pass config load
/// runs, so a rule the agent writes here fails exactly as it would if a
/// human had typed it into the file.
pub async fn set_zones(svc: &Services, rules: Vec<ZoneRuleConfig>) -> Result<ZoneReport, String> {
    let validated = oxplow_config::validate_zone_rules(&rules).map_err(|e| e.to_string())?;
    let for_report = validated.clone();
    crate::config_service::mutate_config(&svc.config, &svc.layout.project_dir, move |c| {
        c.zones = validated;
    })
    .map_err(|e| e.to_string())?;
    svc.events.emit(crate::OxplowEvent::ConfigChanged);
    report_for(svc, for_report).await
}

async fn report_for(svc: &Services, rules: Vec<ZoneRuleConfig>) -> Result<ZoneReport, String> {
    let filter = {
        let cfg = svc.config.read().unwrap_or_else(|e| e.into_inner());
        oxplow_fs_watch::WorkspaceFilter::for_project(
            &svc.layout.project_dir,
            &cfg.generated.exclude,
            &cfg.generated.include,
        )
    };
    let files = svc
        .git
        .list_workspace_files(None, filter)
        .await
        .map_err(|e| e.to_string())?;

    let compiled = ZoneRules::from_config(&rules);
    let mut distribution: BTreeMap<String, usize> = BTreeMap::new();
    let mut unmatched_sample = Vec::new();
    for file in &files {
        let zone = compiled.classify(&file.path);
        if zone == ZONE_OTHER && unmatched_sample.len() < UNMATCHED_SAMPLE {
            unmatched_sample.push(file.path.clone());
        }
        *distribution.entry(zone).or_insert(0) += 1;
    }
    Ok(ZoneReport {
        rules,
        distribution,
        total_files: files.len(),
        unmatched_sample,
    })
}
