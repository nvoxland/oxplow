//! What each kind of contribution asks to do, derived from what it
//! declares (`oxplow_domain::grant`, `.context/extensions.md` "Grants"),
//! and what this machine can hold a grant to. A request is what a person
//! approves; a run decides each thing it does against it.

use oxplow_config::collectors::{CollectorRuntime, CollectorSpec};
use oxplow_domain::grant::{Enforcement, Grant, GrantRequest, GrantedBy};
use oxplow_domain::run::Authority;

use crate::exec_consent::{ApprovalStore, ProgramKind, ProjectProgram};

/// Where a grant is held here: network egress only where the sandbox
/// confines it (`net_sandbox`).
pub fn enforcement() -> Enforcement {
    Enforcement {
        // The network sandbox still permits local brokers/sockets. A
        // broader process's destinations are not fully confined by it.
        hosts: false,
    }
}

/// The scopes among `needs` (the rest are capabilities or features).
fn scopes(needs: &[String]) -> std::collections::BTreeSet<String> {
    needs
        .iter()
        .filter(|n| oxplow_domain::scope::scope(oxplow_domain::service::need_scope(n)).is_some())
        .cloned()
        .collect()
}

/// An extension collector's request: an exec collector's program, hosts,
/// credentials and environment; a script's services (`needs`, what it
/// declares and its kind implies — `services::collector_needs`).
pub fn collector_request(spec: &CollectorSpec, needs: &[String]) -> GrantRequest {
    let runs_program = spec.runtime == CollectorRuntime::Exec;
    GrantRequest {
        scopes: scopes(needs),
        program: runs_program.then(|| spec.entry.clone().unwrap_or_default()),
        execution: spec.execution,
        hosts: spec.network.iter().cloned().collect(),
        credentials: spec.credentials.iter().cloned().collect(),
        env: spec.env.iter().cloned().collect(),
        ..GrantRequest::default()
    }
}

/// A script that composes commands — an effect, an extension command, an
/// effort policy — asks for the scopes it calls.
pub fn script_request(needs: &[String]) -> GrantRequest {
    GrantRequest {
        scopes: scopes(needs),
        ..GrantRequest::default()
    }
}

/// A project program's request, as Settings → Data → Programs lists it.
pub fn program_request(p: &ProjectProgram) -> GrantRequest {
    let runs_program = matches!(
        p.kind,
        ProgramKind::Collector | ProgramKind::AcpAgent | ProgramKind::Provider
    );
    GrantRequest {
        scopes: scopes(&p.scopes),
        execution: p.execution,
        program: runs_program.then(|| p.program.clone()),
        hosts: p.network.iter().cloned().collect(),
        credentials: p.credentials.iter().cloned().collect(),
        // An ACP agent's are `NAME=value`: the name is what it reads.
        env: p
            .env
            .iter()
            .map(|e| e.split('=').next().unwrap_or(e).to_string())
            .collect(),
        commands: p.commands.iter().cloned().collect(),
    }
}

/// What granting `request` lets it do that this machine can't hold it to,
/// as a person reads it — what Programs and a review show beside the
/// grant.
pub fn gaps(request: &GrantRequest) -> Vec<String> {
    let here = enforcement();
    let mut out = Vec::new();
    if request.program.is_some() {
        if request.execution == oxplow_domain::grant::ExecutionTier::Restricted {
            out.push("restricted external execution is unavailable on this platform".into());
        }
        if let Some(why) = here.gap(&oxplow_domain::grant::Need::Files) {
            out.push(why.to_string());
        }
    }
    if let Some(host) = request.hosts.iter().next() {
        if let Some(why) = here.gap(&oxplow_domain::grant::Need::Host(host.clone())) {
            out.push(why.to_string());
        }
    }
    out
}

/// The authority a run of a contribution declaring `declared` has: its
/// grant when a person's approval of `key` holds now (`approved`), else
/// its declaration alone.
pub fn authority(
    approvals: &ApprovalStore,
    key: &str,
    approved: bool,
    declared: GrantRequest,
) -> Authority {
    let grant = approved.then(|| Grant {
        request: declared.clone(),
        granted_at: approvals
            .granted_at(key)
            .and_then(|t| oxplow_domain::Timestamp::parse(&t).ok())
            .unwrap_or_else(oxplow_domain::Timestamp::now),
        granted_by: GrantedBy::Human,
        revoked_at: None,
    });
    Authority {
        declared,
        grant,
        enforcement: enforcement(),
    }
}

/// Carry a run's model authority into SQLite preparation. The policy is
/// host-owned and covers nested and materialized model references.
pub fn bound_query(run: &oxplow_domain::run::RunContext, query: &mut oxplow_db::SqlQuery) {
    if run.has_model_bounds() {
        let run = run.clone();
        query.model_policy = Some(oxplow_db::semantic_layer::ModelReadPolicy::new(
            move |model| match run.decide(oxplow_domain::grant::Need::Model(model.into())) {
                oxplow_domain::grant::Decision::Allowed => Ok(()),
                oxplow_domain::grant::Decision::Denied(reason)
                | oxplow_domain::grant::Decision::Unenforced(reason) => Err(reason),
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A script's request is the scopes it needs, not the capabilities.
    #[test]
    fn a_scripts_request_is_its_scopes() {
        let r = script_request(&["sql.read".into(), "work_items.comments".into()]);
        assert_eq!(r.scopes, ["sql.read".to_string()].into());
        assert!(r.program.is_none());
    }
}
