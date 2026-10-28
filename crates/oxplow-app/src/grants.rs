//! What each kind of contribution asks to do, derived from what it
//! declares (`oxplow_domain::grant`, `.context/extensions.md` "Grants"),
//! what its extension may do (its `permissions:`, approved when it was
//! deployed), and what this machine can hold that to. A run decides each
//! thing it does against them.

use oxplow_config::collectors::{CollectorRuntime, CollectorSpec};
use oxplow_domain::grant::{Enforcement, Grant, GrantRequest};
use oxplow_domain::run::Authority;

use crate::extensions::Extension;

/// Where a grant is held here: network egress only where the sandbox
/// confines it (`net_sandbox`).
pub fn enforcement() -> Enforcement {
    Enforcement {
        // The network sandbox still permits local brokers/sockets. A
        // program's destinations are not fully confined by it.
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

/// What permitting `request` lets it do that this machine can't hold it
/// to, as a person reads it — what a review shows beside it.
pub fn gaps(request: &GrantRequest) -> Vec<String> {
    let here = enforcement();
    let mut out = Vec::new();
    if request.program.is_some() {
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

/// What `ext` may reach: its permissions, and its own commands.
pub fn grant_of(ext: &Extension) -> Grant {
    Grant::from_permissions(&ext.namespace, &ext.permissions)
}

/// The authority a run of one of `ext`'s contributions declaring
/// `declared` has.
pub fn authority(ext: &Extension, declared: GrantRequest) -> Authority {
    Authority {
        declared,
        grant: grant_of(ext),
        enforcement: enforcement(),
    }
}

/// Carry a run's dataset authority into SQLite preparation. The policy is
/// host-owned and covers nested and materialized dataset references.
pub fn bound_query(run: &oxplow_domain::run::RunContext, query: &mut oxplow_db::SqlQuery) {
    if run.has_dataset_bounds() {
        let run = run.clone();
        query.dataset_policy = Some(oxplow_db::semantic_layer::DatasetReadPolicy::new(
            move |dataset| match run.decide(oxplow_domain::grant::Need::Dataset(dataset.into())) {
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
