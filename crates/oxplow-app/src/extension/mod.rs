//! Extensions: loading, activation, deployment, jobs, resources and the rest of the
//! extension machinery. `pub(crate)` by default; a module is `pub` only when
//! another crate imports it (`.context/working-in-this-repo.md`).

#[cfg(test)]
mod contribution_run_pin_tests;
#[cfg(test)]
mod contribution_run_tests;
pub(crate) mod extension_activation;
#[cfg(test)]
mod extension_activation_tests;
pub(crate) mod extension_build;
pub mod extension_catalog;
pub mod extension_commands;
#[cfg(test)]
mod extension_config_tests;
pub(crate) mod extension_configuration;
pub(crate) mod extension_datasets;
pub mod extension_deploy;
pub(crate) mod extension_event_types;
pub(crate) mod extension_generation;
#[cfg(test)]
mod extension_grants_tests;
pub mod extension_impact;
#[cfg(test)]
mod extension_imports_tests;
#[cfg(test)]
mod extension_lifecycle_tests;
#[cfg(test)]
pub(crate) mod extension_private_state_tests;
pub mod extension_resources;
pub(crate) mod extension_runtime_adapters;
pub(crate) mod extension_secrets;
pub(crate) mod extension_services;
pub(crate) mod extension_settings;
pub(crate) mod job_run_ai;
pub(crate) mod job_run_authority;
#[cfg(test)]
mod job_run_crash_tests;
pub mod job_run_dispatch;
#[cfg(test)]
mod job_run_hardening_tests;
pub mod job_run_operations;
pub(crate) mod job_run_runner;
pub(crate) mod job_run_scheduler;
#[cfg(test)]
mod job_run_stream_tests;
pub(crate) mod job_runs;
#[cfg(test)]
mod job_runs_tests;
/// Where an extension's own SQLite file is (a fixture writes it from outside).
pub use extension_services::store_path;
#[cfg(test)]
mod extension_sql_config_tests;
pub(crate) mod extension_state;
pub mod extension_store;
#[cfg(test)]
mod extension_store_services_tests;
pub mod extensions;
pub mod language_set;
mod stray_folder;
