//! The VCS capability's providers (`oxplow_domain::vcs::Vcs`,
//! `.context/vcs.md`). Git is the one built in.

mod git;

pub use git::GitProvider;
