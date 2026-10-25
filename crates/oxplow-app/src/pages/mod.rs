//! Pages: oxplow's one unit of UI and of answers — a page's metadata, its
//! named loaders (`data:`), and the tree of nodes that shows them
//! (`view:`). See `.context/pages-and-tabs.md` "Page files".

pub mod runner;
pub mod schema;

#[cfg(test)]
mod runner_tests;
#[cfg(test)]
mod schema_tests;
