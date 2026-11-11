//! oxwiki: oxplow's own knowledge store (`.context/knowledge.md` "oxwiki").
//! A page is a markdown file, `.oxplow/oxwiki/<slug>.md`, named by its slug
//! (kebab-case). The store keeps the file ([`OxwikiStore`]); the record —
//! the page's row, links and pins — is core's (`oxplow_db::knowledge_record`).
//! A hand edit of a file converges through [`pages::sync_page`], driven by
//! [`watch::follow`].

pub mod pages;
pub mod watch;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use oxplow_domain::events::Envelope;
use oxplow_domain::knowledge::{
    Kept, KindDecl, KnowledgeFeatures, KnowledgeProvider, PageCall, PageDraft, OXWIKI, OXWIKI_DIR,
};
use oxplow_domain::CommandError;

/// oxwiki's folder in a project.
pub fn oxwiki_dir(project_dir: &Path) -> PathBuf {
    project_dir.join(OXWIKI_DIR)
}

/// Where a page's file is.
pub fn page_path(project_dir: &Path, slug: &str) -> PathBuf {
    oxwiki_dir(project_dir).join(format!("{slug}.md"))
}

/// The page whose file `path` (absolute) is: `<project>/.oxplow/oxwiki/<slug>.md`.
pub fn page_of(project_dir: &Path, path: &Path) -> Option<String> {
    let name = path.strip_prefix(oxwiki_dir(project_dir)).ok()?;
    let slug = name.to_str()?.strip_suffix(".md")?;
    valid_slug(slug).then(|| slug.to_string())
}

/// A slug is kebab-case: lowercase letters and digits in `-`-joined runs.
pub fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// The slug a title makes: its letters and digits, lowercased, every other
/// run of characters one `-`. `None` when nothing is left.
pub fn slugify(title: &str) -> Option<String> {
    let mut out = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let slug = out.trim_end_matches('-').to_string();
    valid_slug(&slug).then_some(slug)
}

fn invalid(field: &str, message: String) -> CommandError {
    CommandError::Invalid {
        field: Some(field.into()),
        message,
    }
}

/// The slug a draft writes: its id, or the one its title makes.
pub fn slug_of(draft: &PageDraft) -> Result<String, CommandError> {
    match (&draft.id, &draft.title) {
        (Some(id), _) if valid_slug(id) => Ok(id.clone()),
        (Some(id), _) => Err(invalid(
            "/id",
            format!("`{id}` isn't an oxwiki page id (kebab-case: `some-page`)"),
        )),
        (None, Some(title)) => slugify(title).ok_or_else(|| {
            invalid(
                "/title",
                format!("`{title}` makes no page id: give an `id` (kebab-case)"),
            )
        }),
        (None, None) => Err(invalid(
            "/id",
            "an oxwiki page needs an `id` (kebab-case) or a `title` to make one from".into(),
        )),
    }
}

/// Whether oxwiki is one of the project's enabled stores now: its file
/// watch and its repair (`oxplow.knowledge.resync`) run only then. Turned
/// off, its pages stay recorded, hidden, and its files are left alone.
/// Its appearing is what makes [`watch::follow`] scan its files.
#[derive(Debug, Default)]
pub struct OxwikiPresence {
    present: AtomicBool,
    appeared: tokio::sync::Notify,
}

impl OxwikiPresence {
    /// Set whether it's present; whether it just appeared (and the watch
    /// was told to scan).
    pub fn set(&self, present: bool) -> bool {
        let appeared = !self.present.swap(present, Ordering::SeqCst) && present;
        if appeared {
            self.appeared.notify_one();
        }
        appeared
    }

    pub fn present(&self) -> bool {
        self.present.load(Ordering::SeqCst)
    }

    /// Until it next appears (at once when it appeared since the last call).
    pub async fn appeared(&self) {
        self.appeared.notified().await;
    }
}

/// oxwiki as a store: it writes and deletes a page's file.
pub struct OxwikiStore {
    pub project_dir: PathBuf,
}

#[async_trait]
impl KnowledgeProvider for OxwikiStore {
    fn id(&self) -> &str {
        OXWIKI
    }

    fn kinds(&self) -> Vec<KindDecl> {
        vec![KindDecl::note()]
    }

    fn features(&self) -> KnowledgeFeatures {
        KnowledgeFeatures { edit: true }
    }

    fn page_at(&self, path: &Path) -> Option<String> {
        page_of(&self.project_dir, path)
    }

    async fn write_page(
        &self,
        _call: &PageCall<'_>,
        draft: &PageDraft,
    ) -> Result<Kept, CommandError> {
        let slug = slug_of(draft)?;
        let path = page_path(&self.project_dir, &slug);
        // A page the title names is new: one already there is replaced only
        // when the write names it.
        if draft.id.is_none() && path.exists() {
            return Err(invalid(
                "/id",
                format!("oxwiki already has a page `{slug}`: pass `id: {slug}` to replace it, or another title"),
            ));
        }
        write_file(&path, &draft.body)?;
        Ok(Kept {
            id: slug,
            body: draft.body.clone(),
            title: None,
            kind: None,
            url: None,
            author: None,
            refs: None,
            events: Vec::new(),
        })
    }

    async fn delete_page(
        &self,
        _call: &PageCall<'_>,
        id: &str,
    ) -> Result<Vec<Envelope>, CommandError> {
        if !valid_slug(id) {
            return Err(invalid("/id", format!("`{id}` isn't an oxwiki page id")));
        }
        let path = page_path(&self.project_dir, id);
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(CommandError::Failed {
                message: format!("couldn't delete {}: {e}", path.display()),
            }),
            _ => Ok(Vec::new()),
        }
    }
}

/// Write `body` to a page's file (unless it already holds it), inside the
/// run: the file is the page, so a write that fails fails the run and the
/// transaction records nothing. A temp file renamed into place means the
/// file is never half-written; were the commit itself to fail after it,
/// the file is ahead of the row and the watcher converges them.
pub fn write_file(path: &Path, body: &str) -> Result<(), CommandError> {
    if std::fs::read_to_string(path).ok().as_deref() == Some(body) {
        return Ok(());
    }
    let failed = |e: std::io::Error| CommandError::Failed {
        message: format!("couldn't write {}: {e}", path.display()),
    };
    let dir = path.parent().ok_or_else(|| CommandError::Failed {
        message: format!("{} has no directory", path.display()),
    })?;
    std::fs::create_dir_all(dir).map_err(failed)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let temp = dir.join(format!(".{name}.tmp"));
    std::fs::write(&temp, body)
        .and_then(|()| std::fs::rename(&temp, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&temp);
            failed(e)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_makes_a_slug() {
        assert_eq!(
            slugify("Auth flow: the OAuth callback!").as_deref(),
            Some("auth-flow-the-oauth-callback")
        );
        assert_eq!(slugify("  VCS  ").as_deref(), Some("vcs"));
        assert_eq!(slugify("—"), None);
    }

    #[test]
    fn a_draft_names_its_slug_or_its_title_makes_one() {
        let draft = |id: Option<&str>, title: Option<&str>| PageDraft {
            id: id.map(str::to_string),
            title: title.map(str::to_string),
            ..PageDraft::default()
        };
        assert_eq!(slug_of(&draft(Some("arch"), Some("X"))).unwrap(), "arch");
        assert_eq!(
            slug_of(&draft(None, Some("Auth Flow"))).unwrap(),
            "auth-flow"
        );
        for bad in [
            draft(Some("Not A Slug"), None),
            draft(None, None),
            draft(None, Some("!")),
        ] {
            assert!(matches!(slug_of(&bad), Err(CommandError::Invalid { .. })));
        }
    }

    #[test]
    fn a_page_is_its_file_in_the_folder() {
        let project = Path::new("/p");
        assert_eq!(
            page_of(project, Path::new("/p/.oxplow/oxwiki/arch.md")).as_deref(),
            Some("arch")
        );
        for not in [
            "/p/.oxplow/oxwiki/sub/arch.md",
            "/p/.oxplow/oxwiki/Arch.md",
            "/p/src/arch.md",
            "/p/.oxplow/oxwiki/arch.txt",
        ] {
            assert_eq!(page_of(project, Path::new(not)), None, "{not}");
        }
    }

    #[test]
    fn presence_says_when_it_appears() {
        let p = OxwikiPresence::default();
        assert!(p.set(true), "appeared");
        assert!(!p.set(true), "already there");
        assert!(!p.set(false));
        assert!(!p.present());
    }
}
