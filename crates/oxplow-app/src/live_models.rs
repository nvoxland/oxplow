//! Live models (`.context/semantic-layer.md` "Live models"): published
//! models whose rows are read from live state when a query names them,
//! because that state has no change hook to restate it from — the keychain
//! (is a key set), `ai.yaml` and `project.yaml` (edited by hand), the
//! language-server process table and the binaries on disk, a machine's
//! approvals. A restated table would be wrong between the change and the
//! next restatement; reading when asked is always right, and each read is
//! one small registry walk (no query, no lock held).
//!
//! The mechanism is `metric_grid()`'s: the gateway, before it takes a
//! connection, reads each live model its SQL names and hands the rows to
//! the query as a temp table, with a temp view of the model's own name over
//! it. The core model of that name is an empty typed stub, there for the
//! contract (columns, docs, `v_model`) and to compile against; no model may
//! read one (it would read the stub).

use std::sync::{Arc, OnceLock};

use oxplow_db::{SqlCell, TempTable, TempView};
use oxplow_domain::DomainError;

use crate::ai_service::AiService;
use crate::lsp_sessions::LspSessionManager;

/// The live models, by name (`v_<name>`).
pub const NAMES: &[&str] = &[
    "ai_role",
    "ai_provider",
    "lsp_server",
    "collector_state",
    "collector_credential",
    "zone",
    "zone_unmatched",
];

/// The columns a live model's rows have, in order.
pub fn columns(name: &str) -> &'static [&'static str] {
    match name {
        "ai_role" => &[
            "role",
            "description",
            "extension",
            "fallback",
            "provider",
            "model",
            "restrict",
            "effective_provider",
            "effective_model",
            "key_set",
            "overridden",
            "problem",
        ],
        "ai_provider" => &["id", "kind", "base_url", "key_set"],
        "lsp_server" => &[
            "language_id",
            "extensions",
            "command",
            "args",
            "source",
            "package_name",
            "version",
            "binary_exists",
            "running",
            "running_streams",
            "completion_trigger_characters",
        ],
        "collector_state" => &[
            "ref",
            "owner",
            "id",
            "approved",
            "network_enforced",
            "version",
            "disabled",
        ],
        "collector_credential" => &["collector", "name", "is_set"],
        "zone" => &["position", "zone", "patterns", "color", "files"],
        "zone_unmatched" => &["path"],
        _ => &[],
    }
}

/// What the live models read, bound once the app has built it all.
pub struct LiveSources {
    pub ai: Arc<AiService>,
    pub lsp: LspSessionManager,
    pub worktrees: Arc<crate::worktrees::WorktreeRouter>,
    pub project: String,
    pub approvals: Arc<crate::exec_consent::ApprovalStore>,
    pub collector_store: Arc<oxplow_db::SqliteCollectorStore>,
    pub db: oxplow_db::Database,
    pub vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
    pub secrets: Arc<dyn oxplow_ai::secrets::SecretStore>,
    pub catalog: Arc<crate::extension_catalog::ExtensionCatalog>,
    pub config: Arc<std::sync::RwLock<oxplow_config::OxplowConfig>>,
    pub workspace_files: Arc<crate::workspace_files::WorkspaceFiles>,
}

/// How many unmatched paths `v_zone_unmatched` lists: enough to see the
/// shape of what's missing, few enough to read.
const UNMATCHED_SAMPLE: usize = 20;

/// The gateway's handle on the live models: shared by every clone of it,
/// bound once.
#[derive(Default)]
pub struct LiveModels {
    sources: OnceLock<LiveSources>,
}

fn text(value: impl Into<String>) -> SqlCell {
    SqlCell::Text(value.into())
}

fn opt(value: Option<impl Into<String>>) -> SqlCell {
    value.map_or(SqlCell::Null(()), text)
}

fn flag(value: bool) -> SqlCell {
    SqlCell::Int(i64::from(value))
}

fn json(value: &impl serde::Serialize) -> SqlCell {
    text(serde_json::to_string(value).unwrap_or_default())
}

impl LiveModels {
    pub fn bind(&self, sources: LiveSources) {
        // Bound once, at build; a second bind has nothing to add.
        let _ = self.sources.set(sources);
    }

    pub fn is_bound(&self) -> bool {
        self.sources.get().is_some()
    }

    /// The live models `sql` names (`v_<name>`, bare or quoted).
    pub fn named_in(sql: &str) -> Result<Vec<&'static str>, DomainError> {
        use oxplow_db::sql_tokens::{tokenize, TokenKind};
        let mut found: Vec<&'static str> = Vec::new();
        for token in tokenize(sql)? {
            let word = match token.kind {
                TokenKind::Word => token.text,
                TokenKind::QuotedIdent => token.text.trim_matches(|c| "\"`[]".contains(c)),
                _ => continue,
            };
            let Some(model) = word.strip_prefix("v_").or_else(|| word.strip_prefix("V_")) else {
                continue;
            };
            if let Some(name) = NAMES.iter().find(|n| n.eq_ignore_ascii_case(model)) {
                if !found.contains(name) {
                    found.push(name);
                }
            }
        }
        Ok(found)
    }

    /// The temp table and view that stand for each of `names` in a query:
    /// the rows read now (`with_rows`), or none for a check, which only
    /// compiles.
    pub async fn materialize(
        &self,
        names: &[&'static str],
        with_rows: bool,
    ) -> Result<(Vec<TempTable>, Vec<TempView>), DomainError> {
        let mut tables = Vec::new();
        let mut views = Vec::new();
        for name in names {
            let rows = if with_rows {
                self.read(name).await?
            } else {
                Vec::new()
            };
            let table = format!("live_{name}");
            views.push(TempView {
                name: format!("v_{name}"),
                sql: format!("SELECT * FROM temp.\"{table}\""),
            });
            tables.push(TempTable {
                name: table,
                columns: columns(name).iter().map(|c| c.to_string()).collect(),
                rows,
            });
        }
        Ok((tables, views))
    }

    async fn read(&self, name: &str) -> Result<Vec<Vec<SqlCell>>, DomainError> {
        let sources = self.sources.get().ok_or_else(|| {
            DomainError::Invalid(format!(
                "v_{name} is read live, and this query runs where nothing can read it"
            ))
        })?;
        match name {
            "ai_role" => sources.ai_roles().await,
            "ai_provider" => sources.ai_providers().await,
            "lsp_server" => Ok(sources.lsp_servers().await),
            "collector_state" => sources.collector_state().await,
            "collector_credential" => sources.collector_credentials().await,
            "zone" => Ok(sources.zones().await?.0),
            "zone_unmatched" => Ok(sources.zones().await?.1),
            other => Err(DomainError::Invalid(format!(
                "`v_{other}` isn't a live model"
            ))),
        }
    }
}

impl LiveSources {
    async fn ai_roles(&self) -> Result<Vec<Vec<SqlCell>>, DomainError> {
        let settings = self
            .ai
            .settings()
            .await
            .map_err(|e| DomainError::Invalid(e.to_string()))?;
        let key_set = |provider: &str| {
            settings
                .providers
                .iter()
                .any(|p| p.id == provider && p.key_set)
        };
        Ok(settings
            .roles
            .iter()
            .map(|r| {
                let own = r.binding.as_ref();
                let effective = r.effective.as_ref();
                vec![
                    text(r.role.to_string()),
                    text(r.description.clone()),
                    opt(r.extension.clone()),
                    opt(r.fallback.map(|f| f.name())),
                    opt(own.map(|b| b.provider.clone())),
                    opt(own.map(|b| b.model.clone())),
                    own.map_or(SqlCell::Null(()), |b| json(&b.restrict)),
                    opt(effective.map(|b| b.provider.clone())),
                    opt(effective.map(|b| b.model.clone())),
                    flag(effective.is_some_and(|b| key_set(&b.provider))),
                    flag(r.overridden),
                    opt(r.problem.clone()),
                ]
            })
            .collect())
    }

    async fn ai_providers(&self) -> Result<Vec<Vec<SqlCell>>, DomainError> {
        let settings = self
            .ai
            .settings()
            .await
            .map_err(|e| DomainError::Invalid(e.to_string()))?;
        Ok(settings
            .providers
            .iter()
            .map(|p| {
                vec![
                    text(p.id.clone()),
                    text(p.kind.clone()),
                    opt(p.base_url.clone()),
                    flag(p.key_set),
                ]
            })
            .collect())
    }

    async fn lsp_servers(&self) -> Vec<Vec<SqlCell>> {
        self.lsp
            .list_servers()
            .await
            .iter()
            .map(|s| {
                vec![
                    text(s.language_id.clone()),
                    json(&s.extensions),
                    text(s.command.clone()),
                    json(&s.args),
                    text(match s.source {
                        crate::lsp_sessions::LspServerSource::Yaml => "yaml",
                        crate::lsp_sessions::LspServerSource::Installed => "installed",
                    }),
                    opt(s.package_name.clone()),
                    opt(s.version.clone()),
                    flag(s.binary_exists),
                    SqlCell::Int(s.running_streams.len() as i64),
                    json(&s.running_streams),
                    s.completion_trigger_characters
                        .as_ref()
                        .map_or(SqlCell::Null(()), json),
                ]
            })
            .collect()
    }

    async fn listing(&self) -> Result<Vec<crate::collector_runner::CollectorListing>, DomainError> {
        let root = crate::collector_runner::primary_root(&self.worktrees).await;
        crate::collector_runner::list_with(&crate::collector_runner::Listing {
            root: &root,
            project: &self.project,
            approvals: &self.approvals,
            store: &self.collector_store,
            db: self.db.clone(),
            vocabulary: self.vocabulary.clone(),
            secrets: self.secrets.as_ref(),
            catalog: &self.catalog,
        })
        .await
    }

    async fn collector_state(&self) -> Result<Vec<Vec<SqlCell>>, DomainError> {
        Ok(self
            .listing()
            .await?
            .iter()
            .map(|l| {
                vec![
                    text(oxplow_domain::refs::build::collector_ref(
                        &l.owner, &l.spec.id,
                    )),
                    text(l.owner.clone()),
                    text(l.spec.id.clone()),
                    flag(l.approved),
                    flag(l.network_enforced),
                    opt(l.version.clone()),
                    opt(l.disabled.clone()),
                ]
            })
            .collect())
    }

    async fn collector_credentials(&self) -> Result<Vec<Vec<SqlCell>>, DomainError> {
        Ok(self
            .listing()
            .await?
            .iter()
            .flat_map(|l| {
                let collector = oxplow_domain::refs::build::collector_ref(&l.owner, &l.spec.id);
                l.credentials
                    .iter()
                    .map(move |c| vec![text(collector.clone()), text(c.name.clone()), flag(c.set)])
            })
            .collect())
    }

    /// The zone table's rows and the unmatched sample, from one pass over
    /// the primary worktree's files. Each rule is compiled alone, so a
    /// file is claimed by the first rule that matches it, as
    /// `ZoneRules::classify` decides.
    async fn zones(&self) -> Result<(Vec<Vec<SqlCell>>, Vec<Vec<SqlCell>>), DomainError> {
        let (rules, generated) = {
            let config = self.config.read().unwrap_or_else(|e| e.into_inner());
            (config.zones.clone(), config.generated.clone())
        };
        let files = self
            .workspace_files
            .list_files(None, &generated)
            .await
            .map_err(|e| DomainError::Invalid(e.to_string()))?;
        let singles: Vec<oxplow_code_deps::ZoneRules> = rules
            .iter()
            .map(|r| oxplow_code_deps::ZoneRules::from_config(std::slice::from_ref(r)))
            .collect();
        let mut claimed = vec![0_i64; rules.len()];
        let mut unmatched = Vec::new();
        let mut other = 0_i64;
        for file in &files {
            let hit = singles
                .iter()
                .position(|r| r.classify(&file.path) != oxplow_code_deps::ZONE_OTHER);
            match hit {
                Some(i) => claimed[i] += 1,
                None => {
                    other += 1;
                    if unmatched.len() < UNMATCHED_SAMPLE {
                        unmatched.push(vec![text(file.path.clone())]);
                    }
                }
            }
        }
        let mut table: Vec<Vec<SqlCell>> = rules
            .iter()
            .zip(&claimed)
            .enumerate()
            .map(|(i, (rule, files))| {
                vec![
                    SqlCell::Int(i as i64 + 1),
                    text(rule.zone.clone()),
                    json(&rule.patterns),
                    opt(rule.color.clone()),
                    SqlCell::Int(*files),
                ]
            })
            .collect();
        table.push(vec![
            SqlCell::Null(()),
            text(oxplow_code_deps::ZONE_OTHER),
            text("[]"),
            SqlCell::Null(()),
            SqlCell::Int(other),
        ]);
        Ok((table, unmatched))
    }
}
