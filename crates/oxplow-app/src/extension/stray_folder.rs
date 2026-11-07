//! The folder no extension source may ship: `node_modules` holds a build's
//! tools, not what the extension is. Left in a source it is copied into every
//! deployment and, for an extension that ships with oxplow, compiled into the
//! binary and written to every project's store at every start. One rule, used
//! by the store's walk and by `build.rs`, which includes this file.

/// Why folder `rel` (relative to the source, one its `build.exclude` doesn't
/// name) can't be part of a source; `None` when it can.
pub(crate) fn refusal(rel: &str) -> Option<String> {
    let is_tools = rel.rsplit('/').next() == Some("node_modules");
    is_tools.then(|| {
        format!(
            "`{rel}` is in the source folder: delete it, or list it in the manifest's \
             `build.exclude` if a build uses it"
        )
    })
}
