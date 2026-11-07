//! Extensions: loading, activation, deployment, jobs, resources and the rest of the
//! extension machinery. `pub(crate)` by default; a module is `pub` only when
//! another crate imports it (`.context/working-in-this-repo.md`).

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
pub(crate) mod extension_job_ai;
pub(crate) mod extension_job_authority;
#[cfg(test)]
mod extension_job_crash_tests;
pub mod extension_job_dispatch;
#[cfg(test)]
mod extension_job_hardening_tests;
pub mod extension_job_operations;
pub(crate) mod extension_job_runner;
pub(crate) mod extension_job_scheduler;
#[cfg(test)]
mod extension_job_stream_tests;
pub(crate) mod extension_jobs;
#[cfg(test)]
mod extension_jobs_tests;
#[cfg(test)]
mod extension_lifecycle_tests;
#[cfg(test)]
mod extension_private_state_tests;
pub mod extension_resources;
#[cfg(test)]
mod extension_run_health_tests;
#[cfg(test)]
mod extension_run_pin_tests;
pub(crate) mod extension_runtime_adapters;
pub(crate) mod extension_secrets;
#[cfg(test)]
mod extension_sql_config_tests;
pub(crate) mod extension_state;
pub mod extension_store;
pub mod extensions;
mod stray_folder;
