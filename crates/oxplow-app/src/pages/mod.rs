//! Pages: oxplow's one unit of UI and of answers — a page's metadata, its
//! named loaders (`data:`), and the tree of nodes that shows them
//! (`view:`). See `.context/pages-and-tabs.md` "Page files".

pub mod actions;
pub mod convert;
pub mod open_runs;
pub mod runner;
pub mod schema;
pub mod text;

#[cfg(test)]
mod actions_tests;
#[cfg(test)]
mod convert_tests;
#[cfg(test)]
mod open_runs_tests;
#[cfg(test)]
mod runner_tests;
#[cfg(test)]
mod schema_tests;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod text_tests;
