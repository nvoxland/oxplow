//! Live models (`.context/semantic-layer.md` "Live models"): published
//! models whose rows are read from live state when a query names them,
//! because that state has no change hook to restate it from — the keychain
//! (is a key set), `ai.yaml` and `project.yaml` (edited by hand), the
//! language-server process table and the binaries on disk, the
//! worktree's files, the harnesses' saved conversations. A
//! restated table would be wrong between the change and the next
//! restatement; reading when asked is always right.
//!
//! A query runs for a stream (`SqlQuery::stream`, a lens's `stream_id`):
//! a model of a worktree — its files, its conversations — reads that
//! stream's, and the primary checkout's when the query names none.
//!
//! What changes a live model is in [`follows`]: the events that change
//! what it reads, and the models it reads through. The change loop
//! announces it as changed with them (`models_changed`), so a lens over
//! it re-runs like one over a table.
//!
//! The mechanism is `metric_grid()`'s: the gateway, before it takes a
//! connection, reads each live model its SQL names and hands the rows to
//! the query as a temp table, with a temp view of the model's own name over
//! it. The core model of that name is an empty typed stub, there for the
//! contract (columns, docs, `v_model`) and to compile against; no model may
//! read one (it would read the stub).

use std::sync::{Arc, OnceLock};

use oxplow_db::{SqlCell, SqlParams, TempTable, TempView};
use oxplow_domain::agent::harness::{
    AgentHarness, ConversationQuery, HarnessError, NativeConversation,
};
use oxplow_domain::{DomainError, Stream, StreamId, StreamKind};

use crate::ai_service::AiService;
use crate::events::OxplowEvent;
use crate::lsp_sessions::LspSessionManager;

/// The live models, by name (`v_<name>`).
pub const NAMES: &[&str] = &[
    "ai_role",
    "ai_provider",
    "lsp_server",
    "collector_state",
    "collector_credential",
    "extension_secret",
    "extension_source",
    "zone",
    "zone_unmatched",
    "scope",
    "config_value",
    "agent_conversation",
    "vcs_workspace",
    "workspace_file",
    "agent_harness",
    "branch_divergence",
    "commit_between",
    "extension_change",
    "metric_point",
    "data_entity",
    "effect_backfill",
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
        "collector_state" => &["uri", "owner", "id", "network_enforced", "disabled"],
        "collector_credential" => &["collector", "name", "is_set"],
        "extension_secret" => &["extension", "provider", "name", "reference", "is_set"],
        "extension_source" => &["name", "path", "hash"],
        "zone" => &["position", "zone", "patterns", "color", "files"],
        "zone_unmatched" => &["path"],
        "scope" => &["id", "access", "host", "spends", "summary"],
        "config_value" => &["key", "value", "origin", "extension", "doc", "schema"],
        "agent_conversation" => &[
            "stream_id",
            "harness",
            "native_id",
            "title",
            "first_prompt",
            "started_at",
            "updated_at",
            "size_bytes",
            "parent",
            "problem",
        ],
        "vcs_workspace" => &["path", "branch", "head", "is_main", "stream_id"],
        "agent_harness" => &["id", "title", "enabled", "position", "settings"],
        "workspace_file" => &["stream_id", "path", "parent", "name", "kind", "status"],
        "branch_divergence" => &[
            "base",
            "head",
            "ahead",
            "behind",
            "merge_base",
            "overlapping_files",
            "readiness",
            "stream_id",
        ],
        "commit_between" => &[
            "base",
            "head",
            "sha",
            "short_sha",
            "author",
            "email",
            "committed_at",
            "subject",
            "stream_id",
        ],
        "extension_change" => &[
            "base",
            "head",
            "name",
            "change",
            "errors",
            "lines",
            "lens_changes",
            "stream_id",
        ],
        "metric_point" => &[
            "metric",
            "dimension",
            "capture_id",
            "captured_at",
            "value",
            "group_value",
            "branch",
            "stream_id",
        ],
        "data_entity" => &["name", "owner", "kind", "description", "row_count"],
        "effect_backfill" => &["effect", "planned", "from_seq", "to_seq", "batch"],
        _ => &[],
    }
}

/// The columns a live model can read narrowed by: a query that says
/// `<column> = <value>` of every row it reads is handed only those rows.
pub fn pushdown(name: &str) -> &'static [&'static str] {
    match name {
        // One folder is one `read_dir`; the whole tree is a walk.
        "workspace_file" => &["parent"],
        // One pair is one graph walk; a side left open ranges over the
        // branches (`LiveSources::branch_pairs`).
        "branch_divergence" | "commit_between" | "extension_change" => &["base", "head"],
        // One metric is one series read; the dimension is how it is sliced.
        "metric_point" => &["metric", "dimension"],
        // Counting a model is a query of its own; counting an effect's
        // backfill reads the event log: each only for the one asked for.
        "data_entity" => &["name"],
        "effect_backfill" => &["effect"],
        _ => &[],
    }
}

/// `sql`'s certain `<column> = <text>` conditions on `name`'s [`pushdown`]
/// columns (`sql_tokens::text_filters`).
fn filters(
    name: &str,
    sql: &str,
    params: &SqlParams,
) -> Result<Vec<(String, String)>, DomainError> {
    let columns = pushdown(name);
    if columns.is_empty() {
        return Ok(Vec::new());
    }
    oxplow_db::sql_tokens::text_filters(sql, &format!("v_{name}"), columns, params)
}

/// What can change a live model's rows.
pub enum Change<'a> {
    /// An event about what it reads.
    Event(&'a OxplowEvent),
    /// These models changed (`v_<name>`).
    Models(&'a [String]),
}

/// Whether `change` can change live model `name`'s rows: the events about
/// the state it reads, and the models it reads through.
pub fn follows(name: &str, change: &Change<'_>) -> bool {
    use OxplowEvent as E;
    match change {
        Change::Event(event) => match name {
            "ai_role" | "ai_provider" => matches!(
                event,
                E::ConfigChanged | E::CredentialChanged { .. } | E::ExtensionsChanged
            ),
            "lsp_server" => matches!(event, E::LspServersChanged | E::ConfigChanged),
            "collector_state" | "collector_credential" | "extension_secret" => matches!(
                event,
                E::CredentialChanged { .. } | E::ExtensionsChanged | E::ConfigChanged
            ),
            "zone" | "zone_unmatched" => {
                matches!(event, E::ConfigChanged | E::WorkspaceChanged { .. })
            }
            // A source is files in the checkout, and where it lives is config.
            "extension_source" => matches!(
                event,
                E::WorkspaceChanged { .. } | E::ConfigChanged | E::ExtensionsChanged
            ),
            "config_value" | "agent_harness" => {
                matches!(event, E::ConfigChanged | E::ExtensionsChanged)
            }
            // A push, pull, fetch, commit or checkout moves what a pair
            // compares.
            "vcs_workspace" | "branch_divergence" | "commit_between" => {
                matches!(event, E::VcsRefsChanged { .. })
            }
            // A range headed at the working tree moves with the files.
            "extension_change" => matches!(
                event,
                E::VcsRefsChanged { .. } | E::WorkspaceChanged { .. } | E::ExtensionsChanged
            ),
            "workspace_file" => matches!(
                event,
                E::WorkspaceChanged { .. } | E::VcsRefsChanged { .. } | E::ConfigChanged
            ),
            // The catalog and the config decide what exists; a model's
            // rows are counted when asked for.
            "data_entity" | "effect_backfill" => {
                matches!(event, E::ConfigChanged | E::ExtensionsChanged)
            }
            // New captures, and the metrics a config or extension turns on.
            "metric_point" => matches!(
                event,
                E::MetricSamplesChanged { .. } | E::ConfigChanged | E::ExtensionsChanged
            ),
            _ => false,
        },
        Change::Models(models) => {
            let reads = |m: &str| models.iter().any(|x| x == m);
            match name {
                // Its conversations change as sessions run.
                "agent_conversation" => reads("v_agent_session") || reads("v_agent_turn"),
                // A stream adopting a working copy.
                "vcs_workspace" => reads("v_stream"),
                // The branches an unpinned side ranges over.
                "branch_divergence" | "commit_between" => reads("v_branch"),
                // The metrics it lists: the catalog and the specs.
                "metric_point" => reads("v_metric_catalog") || reads("v_metric_spec"),
                _ => false,
            }
        }
    }
}

/// What the live models read, bound once the app has built it all.
pub struct LiveSources {
    pub ai: Arc<AiService>,
    pub lsp: LspSessionManager,
    pub worktrees: Arc<crate::worktrees::WorktreeRouter>,
    pub project: String,
    pub collector_store: Arc<oxplow_db::SqliteCollectorStore>,
    pub db: oxplow_db::Database,
    pub vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
    pub secrets: Arc<dyn oxplow_ai::secrets::SecretStore>,
    pub catalog: Arc<crate::extension_catalog::ExtensionCatalog>,
    pub config: Arc<std::sync::RwLock<oxplow_config::OxplowConfig>>,
    pub workspace_files: Arc<crate::workspace_files::WorkspaceFiles>,
    pub streams: Arc<dyn oxplow_domain::stores::StreamStore>,
    pub harnesses: oxplow_domain::agent::registry::HarnessRegistry,
    pub vcs: Arc<dyn oxplow_domain::vcs::Vcs>,
    /// The services, once built (`v_extension_change` reviews with them).
    pub services: crate::commands::util::ServicesSlot,
}

/// The most conversations listed per harness.
const LIMIT: usize = 50;

/// The saved conversations of `harness` begun in `stream`'s worktree.
pub async fn harness_conversations(
    harness: &dyn AgentHarness,
    stream: &Stream,
) -> Result<Vec<NativeConversation>, HarnessError> {
    let query = ConversationQuery {
        cwd: std::path::PathBuf::from(&stream.worktree_path),
        // Its saved conversations are on the machine it runs on.
        home: (stream.host == oxplow_domain::HostId::LOCAL)
            .then(|| std::env::var_os("HOME").map(std::path::PathBuf::from))
            .flatten(),
        search_path: crate::agent_path::search_path(),
        limit: LIMIT,
    };
    harness.conversations(&query).await
}

/// How many unmatched paths `v_zone_unmatched` lists: enough to see the
/// shape of what's missing, few enough to read.
const UNMATCHED_SAMPLE: usize = 20;

/// The most commits `v_commit_between` lists per pair: it answers what is
/// unpushed or unmerged, not a history (`v_commit` is that).
const BETWEEN_LIMIT: u32 = 100;

/// The most captures `v_metric_point` lists per metric when no metric is
/// pinned (a list's sparklines), and when one is (its chart).
const POINTS_UNPINNED: usize = 200;
const POINTS_PINNED: usize = 5000;

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

/// An enum value as its wire name (`default`, `project`).
fn wire_name(value: &impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
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

    /// The temp table and view that stand for each of `names` in `sql`,
    /// run for `stream` (a row id; none: the primary checkout): the rows
    /// read now with the query's `params`, or none for a check (`None`),
    /// which only compiles. A model reads only the rows `sql`'s conditions
    /// on its [`pushdown`] columns keep, when they certainly apply
    /// (`sql_tokens::equality_filters`).
    pub async fn materialize(
        &self,
        names: &[&'static str],
        stream: Option<i64>,
        sql: &str,
        params: Option<&SqlParams>,
    ) -> Result<(Vec<TempTable>, Vec<TempView>), DomainError> {
        let mut tables = Vec::new();
        let mut views = Vec::new();
        let stream = stream.map(StreamId::new);
        for name in names {
            let rows = match params {
                Some(params) => {
                    let filters = filters(name, sql, params)?;
                    self.read(name, stream, &filters).await?
                }
                None => Vec::new(),
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

    async fn read(
        &self,
        name: &str,
        stream: Option<StreamId>,
        filters: &[(String, String)],
    ) -> Result<Vec<Vec<SqlCell>>, DomainError> {
        // A table core ships: nothing to bind.
        if name == "scope" {
            return Ok(scopes());
        }
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
            "extension_secret" => sources.extension_secrets().await,
            "extension_source" => sources.extension_sources().await,
            "zone" => Ok(sources.zones().await?.0),
            "zone_unmatched" => Ok(sources.zones().await?.1),
            "config_value" => sources.config_values().await,
            "agent_conversation" => sources.agent_conversations(stream).await,
            "vcs_workspace" => sources.vcs_workspaces().await,
            "workspace_file" => {
                let parent = filters.iter().find(|(c, _)| c == "parent");
                sources
                    .workspace_files(stream, parent.map(|(_, v)| v.clone()))
                    .await
            }
            "agent_harness" => Ok(sources.agent_harnesses()),
            "branch_divergence" => sources.branch_divergence(stream, filters).await,
            "commit_between" => sources.commits_between(stream, filters).await,
            "extension_change" => sources.extension_changes(stream, filters).await,
            "metric_point" => sources.metric_points(stream, filters).await,
            "data_entity" => sources.data_entities(filters).await,
            "effect_backfill" => sources.effect_backfill(filters).await,
            other => Err(DomainError::Invalid(format!(
                "`v_{other}` isn't a live model"
            ))),
        }
    }
}

/// `v_scope`: every scope a command may need.
fn scopes() -> Vec<Vec<SqlCell>> {
    oxplow_domain::scope::SCOPES
        .iter()
        .map(|s| {
            vec![
                text(s.id),
                text(wire_name(&s.access)),
                text(wire_name(&s.host)),
                SqlCell::Int(i64::from(s.spends)),
                text(s.summary),
            ]
        })
        .collect()
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
                    text(oxplow_domain::uri::build::collector_uri(
                        &l.owner, &l.spec.id,
                    )),
                    text(l.owner.clone()),
                    text(l.spec.id.clone()),
                    flag(l.network_enforced),
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
                let collector = oxplow_domain::uri::build::collector_uri(&l.owner, &l.spec.id);
                l.credentials
                    .iter()
                    .map(move |c| vec![text(collector.clone()), text(c.name.clone()), flag(c.set)])
            })
            .collect())
    }

    /// The credentials the enabled extensions' providers declare with a
    /// host secret reference, and whether the keychain holds a value for
    /// each (presence only: the value is never read out).
    async fn extension_secrets(&self) -> Result<Vec<Vec<SqlCell>>, DomainError> {
        let root = crate::collector_runner::primary_root(&self.worktrees).await;
        let mut rows = Vec::new();
        for extension in self.catalog.get(&root).iter() {
            for provider in &extension.providers {
                for credential in &provider.credentials {
                    let Some(reference) = &credential.secret_ref else {
                        continue;
                    };
                    let account = crate::extension_secrets::account(
                        &self.project,
                        &extension.name,
                        reference,
                    );
                    let is_set = self
                        .secrets
                        .get(&account)
                        .map_err(|_| DomainError::Invariant("reading the keychain failed".into()))?
                        .is_some();
                    rows.push(vec![
                        text(extension.name.clone()),
                        text(provider.id.clone()),
                        text(credential.name.clone()),
                        json(reference),
                        flag(is_set),
                    ]);
                }
            }
        }
        Ok(rows)
    }

    /// Each extension source in the main checkout as it is now: its name,
    /// where it is and its hash (`extension_store::source_hash`, the hash a
    /// deploy of it would have).
    async fn extension_sources(&self) -> Result<Vec<Vec<SqlCell>>, DomainError> {
        let project = self.worktrees.project_dir().to_path_buf();
        tokio::task::spawn_blocking(move || {
            let settings = oxplow_config::extension_settings(&project);
            crate::extensions::known_names(&project, &settings)
                .into_iter()
                .filter_map(|name| {
                    let dir = settings.source(&project, &name);
                    if !dir.join("extension.yaml").is_file() {
                        return None;
                    }
                    let hash = crate::extension_store::source_hash(&dir).ok()?;
                    Some(vec![
                        text(name),
                        text(crate::extensions::shown(&project, &dir)),
                        text(hash),
                    ])
                })
                .collect()
        })
        .await
        .map_err(|e| DomainError::Invariant(format!("reading extension sources: {e}")))
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

    /// The stream a query runs for: `stream`, else the primary.
    async fn stream(&self, stream: Option<StreamId>) -> Result<Stream, DomainError> {
        if let Some(id) = stream {
            return self.streams.get(&id).await?.ok_or(DomainError::NotFound);
        }
        self.streams
            .list()
            .await?
            .into_iter()
            .find(|s| s.kind == StreamKind::Primary)
            .ok_or(DomainError::NotFound)
    }

    async fn config_values(&self) -> Result<Vec<Vec<SqlCell>>, DomainError> {
        let config = self
            .config
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let project_dir = self.worktrees.project_dir();
        let extensions = self
            .catalog
            .unpinned(project_dir, "v_config_value lists what extensions declare");
        let ai = self.ai.settings().await.ok();
        let global = oxplow_config::global_config_dir();
        Ok(crate::effective_config::effective_config(
            &config,
            project_dir,
            ai.as_ref(),
            global.as_deref(),
            extensions.as_ref(),
        )
        .iter()
        .map(|s| {
            vec![
                text(s.key.clone()),
                json(&s.value),
                text(wire_name(&s.origin)),
                opt(s.extension.clone()),
                text(s.doc.clone()),
                if s.schema.is_null() {
                    SqlCell::Null(())
                } else {
                    json(&s.schema)
                },
            ]
        })
        .collect())
    }

    /// Every harness's saved conversations in the stream's worktree, each
    /// asked at once; one that can't answer is a row saying why.
    async fn agent_conversations(
        &self,
        stream: Option<StreamId>,
    ) -> Result<Vec<Vec<SqlCell>>, DomainError> {
        use oxplow_domain::agent::harness::HarnessError;
        let stream = self.stream(stream).await?;
        let harnesses = self.harnesses.all();
        let answers = futures::future::join_all(harnesses.iter().map(|h| {
            let stream = &stream;
            async move {
                (
                    h.id().to_string(),
                    harness_conversations(h.as_ref(), stream).await,
                )
            }
        }))
        .await;
        let id = SqlCell::Int(stream.id.value());
        let time = |t: Option<oxplow_domain::Timestamp>| opt(t.map(|t| t.to_string()));
        let mut rows = Vec::new();
        for (harness, answer) in answers {
            match answer {
                Ok(found) => rows.extend(found.into_iter().map(|c| {
                    vec![
                        id.clone(),
                        text(harness.clone()),
                        text(c.id),
                        opt(c.title),
                        opt(c.first_prompt),
                        time(c.started_at),
                        time(c.updated_at),
                        c.size_bytes
                            .map_or(SqlCell::Null(()), |n| SqlCell::Int(n as i64)),
                        opt(c.parent),
                        SqlCell::Null(()),
                    ]
                })),
                // It keeps no list: nothing to say.
                Err(HarnessError::Unsupported { .. }) => {}
                Err(problem) => {
                    let mut row = vec![SqlCell::Null(()); columns("agent_conversation").len()];
                    row[0] = id.clone();
                    row[1] = text(harness);
                    row[9] = text(problem.to_string());
                    rows.push(row);
                }
            }
        }
        Ok(rows)
    }

    /// The repository's working copies, primary first, each with the stream
    /// working in it (paths compared as the disk resolves them).
    /// The registered harnesses in priority order: those `agents:` names, in
    /// its order (the first is a new session's default), then the rest.
    fn agent_harnesses(&self) -> Vec<Vec<SqlCell>> {
        let agents = crate::config_service::read_config(&self.config).agents;
        let mut all = self.harnesses.all();
        all.sort_by_key(|h| {
            agents
                .iter()
                .position(|a| a == h.id())
                .unwrap_or(agents.len())
        });
        all.iter()
            .enumerate()
            .map(|(position, h)| {
                vec![
                    text(h.id().to_string()),
                    text(h.title().to_string()),
                    flag(agents.is_empty() || agents.iter().any(|a| a == h.id())),
                    SqlCell::Int(position as i64),
                    json(&h.settings()),
                ]
            })
            .collect()
    }

    async fn vcs_workspaces(&self) -> Result<Vec<Vec<SqlCell>>, DomainError> {
        let canon = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.into());
        let project_dir = self.worktrees.project_dir();
        let streams: Vec<(std::path::PathBuf, i64)> = self
            .streams
            .list()
            .await?
            .iter()
            .map(|s| {
                let path = crate::worktrees::workspace_path(project_dir, &s.worktree_path);
                (canon(&path), s.id.value())
            })
            .collect();
        let mut workspaces = self.vcs.list_workspaces(project_dir).await?;
        workspaces.sort_by_key(|w| !w.is_main);
        Ok(workspaces
            .into_iter()
            .map(|w| {
                let at = canon(std::path::Path::new(&w.path));
                let stream = streams.iter().find(|(p, _)| *p == at).map(|(_, id)| *id);
                vec![
                    text(w.path),
                    opt(w.branch),
                    opt(w.head),
                    flag(w.is_main),
                    stream.map_or(SqlCell::Null(()), SqlCell::Int),
                ]
            })
            .collect())
    }

    /// The pairs of revisions `v_branch_divergence` and `v_commit_between`
    /// compare, in `stream`'s worktree, from what the query pins: both
    /// sides, one pair; one side, the other every branch of the repository
    /// (local and remote-tracking, the pinned one left out); neither,
    /// every local branch as the head against the default branch (which is
    /// left out). Each side is kept as the query spelled it (its rows'
    /// `base` and `head`) and resolved beside; a side the VCS can't resolve
    /// drops its pair, so one bad name never empties a dashboard.
    async fn branch_pairs(
        &self,
        stream: Option<StreamId>,
        filters: &[(String, String)],
    ) -> Result<(Stream, std::path::PathBuf, Vec<(String, String)>), DomainError> {
        let stream = self.stream(stream).await?;
        let ws =
            crate::worktrees::workspace_path(self.worktrees.project_dir(), &stream.worktree_path);
        let pinned = |column: &str| {
            filters
                .iter()
                .find(|(c, _)| c == column)
                .map(|(_, v)| v.clone())
        };
        let (base, head) = (pinned("base"), pinned("head"));
        let branches = || async { Ok::<_, DomainError>(self.vcs.branches(&ws).await?) };
        let full_name = |b: &oxplow_domain::vcs::Branch| match &b.remote {
            Some(remote) => format!("{remote}/{}", b.name),
            None => b.name.clone(),
        };
        let pairs: Vec<(String, String)> = match (base, head) {
            (Some(base), Some(head)) => vec![(base, head)],
            (Some(base), None) => branches()
                .await?
                .iter()
                .map(full_name)
                .filter(|h| *h != base)
                .map(|h| (base.clone(), h))
                .collect(),
            (None, Some(head)) => branches()
                .await?
                .iter()
                .map(full_name)
                .filter(|b| *b != head)
                .map(|b| (b, head.clone()))
                .collect(),
            (None, None) => {
                let branches = branches().await?;
                match branches
                    .iter()
                    .find(|b| b.remote.is_none() && b.is_default)
                    .map(|b| b.name.clone())
                {
                    Some(default) => branches
                        .iter()
                        .filter(|b| b.remote.is_none() && b.name != default)
                        .map(|b| (default.clone(), b.name.clone()))
                        .collect(),
                    None => Vec::new(),
                }
            }
        };
        let mut resolved = Vec::with_capacity(pairs.len());
        for (base, head) in pairs {
            let both = (
                self.vcs.resolve(&ws, &base).await,
                self.vcs.resolve(&ws, &head).await,
            );
            if both.0.is_ok() && both.1.is_ok() {
                resolved.push((base, head));
            }
        }
        Ok((stream, ws, resolved))
    }

    /// `v_branch_divergence`: how far each pair's `head` has moved from its
    /// `base` (`Vcs::divergence`).
    async fn branch_divergence(
        &self,
        stream: Option<StreamId>,
        filters: &[(String, String)],
    ) -> Result<Vec<Vec<SqlCell>>, DomainError> {
        let (stream, ws, pairs) = self.branch_pairs(stream, filters).await?;
        let mut rows = Vec::with_capacity(pairs.len());
        for (base, head) in pairs {
            let d = self.vcs.divergence(&ws, &base, &head).await?;
            rows.push(vec![
                text(base),
                text(head),
                SqlCell::Int(i64::from(d.ahead)),
                SqlCell::Int(i64::from(d.behind)),
                opt(d.merge_base),
                json(&d.overlapping_files),
                text(wire_name(&d.readiness)),
                SqlCell::Int(stream.id.value()),
            ]);
        }
        Ok(rows)
    }

    /// `v_commit_between`: for each pair, the commits on `head` that
    /// `base` lacks, newest first (`Vcs::revisions_between`), at most
    /// [`BETWEEN_LIMIT`] of them.
    async fn commits_between(
        &self,
        stream: Option<StreamId>,
        filters: &[(String, String)],
    ) -> Result<Vec<Vec<SqlCell>>, DomainError> {
        let (stream, ws, pairs) = self.branch_pairs(stream, filters).await?;
        let mut rows = Vec::new();
        for (base, head) in pairs {
            for c in self
                .vcs
                .revisions_between(&ws, &base, &head, BETWEEN_LIMIT)
                .await?
            {
                rows.push(vec![
                    text(base.clone()),
                    text(head.clone()),
                    text(c.id),
                    text(c.short_id),
                    text(c.author),
                    text(c.email),
                    text(oxplow_domain::Timestamp::from_unix_ms(c.time * 1000).to_text()),
                    text(c.subject),
                    SqlCell::Int(stream.id.value()),
                ]);
            }
        }
        Ok(rows)
    }

    /// `v_extension_change`: the extensions under `.oxplow/extensions/`
    /// whose files differ between the revisions the query pins as `base`
    /// (`''`: nothing before) and `head`, each with what the change does
    /// (`extensions::extension_changes_between`). Unpinned, nothing: the
    /// review runs both versions.
    async fn extension_changes(
        &self,
        stream: Option<StreamId>,
        filters: &[(String, String)],
    ) -> Result<Vec<Vec<SqlCell>>, DomainError> {
        let pinned = |column: &str| {
            filters
                .iter()
                .find(|(c, _)| c == column)
                .map(|(_, v)| v.clone())
        };
        let (Some(base), Some(head)) = (pinned("base"), pinned("head")) else {
            return Ok(Vec::new());
        };
        let stream = self.stream(stream).await?;
        let ws =
            crate::worktrees::workspace_path(self.worktrees.project_dir(), &stream.worktree_path);
        let parse = |s: &str| {
            s.parse::<oxplow_domain::vcs::Revision>()
                .map_err(DomainError::Invalid)
        };
        let start = if base.is_empty() || base == "none" {
            None
        } else {
            Some(parse(&base)?)
        };
        let end = parse(&head)?;
        let svc = self
            .services
            .get()
            .map_err(|e| DomainError::Invalid(e.to_string()))?;
        let changes =
            crate::extensions::extension_changes_between(&svc, &ws, start.as_ref(), &end).await?;
        Ok(changes
            .into_iter()
            .map(|c| {
                let lines = c
                    .impact
                    .as_ref()
                    .map(|i| i.lines.clone())
                    .unwrap_or_default();
                let lenses: Vec<serde_json::Value> = c
                    .impact
                    .iter()
                    .flat_map(|i| i.lenses.iter())
                    .filter(|l| !matches!(l.change, crate::extension_impact::Change::Unchanged))
                    .map(|l| serde_json::json!({ "id": l.id, "change": wire_name(&l.change) }))
                    .collect();
                vec![
                    text(base.clone()),
                    text(head.clone()),
                    text(c.name),
                    text(wire_name(&c.change)),
                    json(&c.errors),
                    json(&lines),
                    json(&lenses),
                    SqlCell::Int(stream.id.value()),
                ]
            })
            .collect())
    }

    /// `v_metric_point`: one row per capture of a metric, newest first, the
    /// metric's series as the engine reads it (`series_for_spec_read`):
    /// every metric with a spec, or the one the query pins as `metric`,
    /// sliced by the `dimension` it pins (one row per capture and group;
    /// a metric the dimension can't slice has no rows). Unpinned, each
    /// metric's newest [`POINTS_UNPINNED`] captures; pinned, [`POINTS_PINNED`].
    async fn metric_points(
        &self,
        stream: Option<StreamId>,
        filters: &[(String, String)],
    ) -> Result<Vec<Vec<SqlCell>>, DomainError> {
        let pinned = |column: &str| {
            filters
                .iter()
                .find(|(c, _)| c == column)
                .map(|(_, v)| v.clone())
        };
        let (metric, dimension) = (pinned("metric"), pinned("dimension"));
        let svc = self
            .services
            .get()
            .map_err(|e| DomainError::Invalid(e.to_string()))?;
        let keys = match &metric {
            Some(key) => vec![key.clone()],
            None => svc
                .fact_store
                .list_specs()
                .await?
                .into_iter()
                .map(|s| s.key)
                .collect(),
        };
        let cap = if metric.is_some() {
            POINTS_PINNED
        } else {
            POINTS_UNPINNED
        };
        let stream = self.stream(stream).await?.id.value();
        let mut rows = Vec::new();
        for key in keys {
            let Some(spec) = svc.metric_engine.spec(&key).await? else {
                continue;
            };
            let read = crate::metric_engine::SeriesRead {
                group_by: dimension.clone().filter(|d| !d.is_empty()),
                stream: Some(stream),
                ..Default::default()
            };
            // A formula, or a metric the dimension can't slice, has no
            // points here rather than failing the others.
            let Ok(mut points) = svc.metric_engine.series_for_spec_read(&spec, &read).await else {
                continue;
            };
            points.sort_by_key(|p| std::cmp::Reverse(p.captured_at.to_text()));
            for p in points.into_iter().take(cap) {
                rows.push(vec![
                    text(key.clone()),
                    opt(dimension.clone()),
                    SqlCell::Int(p.capture_id),
                    text(p.captured_at.to_text()),
                    SqlCell::Real(p.value),
                    opt(p.group),
                    opt(p.branch),
                    SqlCell::Int(stream),
                ]);
            }
        }
        Ok(rows)
    }

    /// `v_data_entity`: every published model, then every entity the
    /// enabled extensions declare but haven't synced (`kind` `sql`,
    /// `entity` or `declared`). `row_count` is the model's row count, counted
    /// only for the one the query pins as `name` (counting each is a
    /// query: the list leaves it NULL).
    async fn data_entities(
        &self,
        filters: &[(String, String)],
    ) -> Result<Vec<Vec<SqlCell>>, DomainError> {
        let svc = self
            .services
            .get()
            .map_err(|e| DomainError::Invalid(e.to_string()))?;
        let root = self.worktrees.project_dir().to_path_buf();
        let all = crate::semantic_catalog::data_entities(&svc.sql, &self.catalog, &root).await?;
        let pinned = filters.iter().find(|(c, _)| c == "name").map(|(_, v)| v);
        let mut rows = Vec::new();
        for e in all {
            if pinned.is_some_and(|n| *n != e.name) {
                continue;
            }
            let counted = if pinned.is_some() && e.kind != "declared" {
                // Names come from the registry, never from the query.
                let result = svc
                    .sql
                    .query_sql(
                        &format!("SELECT count(*) FROM \"{}\"", e.name),
                        vec![],
                        Some(1),
                    )
                    .await;
                match result
                    .ok()
                    .and_then(|r| r.rows.into_iter().next())
                    .and_then(|r| r.into_iter().next())
                {
                    Some(SqlCell::Int(n)) => SqlCell::Int(n),
                    _ => SqlCell::Null(()),
                }
            } else {
                SqlCell::Null(())
            };
            rows.push(vec![
                text(e.name),
                text(e.owner),
                text(e.kind),
                text(e.description),
                counted,
            ]);
        }
        Ok(rows)
    }

    /// `v_effect_backfill`: what `oxplow.effect.backfill` would have the
    /// effect pinned as `effect` (`<extension>/<id>`) react to: how many
    /// events, the log positions they span, and how many one run reacts
    /// to. Unpinned, nothing: reading the log for each effect is a scan.
    async fn effect_backfill(
        &self,
        filters: &[(String, String)],
    ) -> Result<Vec<Vec<SqlCell>>, DomainError> {
        let Some((_, name)) = filters.iter().find(|(c, _)| c == "effect") else {
            return Ok(Vec::new());
        };
        let svc = self
            .services
            .get()
            .map_err(|e| DomainError::Invalid(e.to_string()))?;
        let Some((_, decl)) = crate::effect_triggers::find_effect(&svc, name) else {
            return Ok(Vec::new());
        };
        let vocabulary = svc.vocabulary.current();
        let planned = svc
            .db
            .read(move |tx| {
                crate::commands::effect::unreacted_tx(
                    tx,
                    &vocabulary,
                    &decl,
                    &crate::commands::effect::Range::default(),
                    0,
                    1_000,
                )
            })
            .await?;
        let opt = |v: Option<i64>| v.map_or(SqlCell::Null(()), SqlCell::Int);
        Ok(vec![vec![
            text(name.clone()),
            SqlCell::Int(i64::try_from(planned.count).unwrap_or(i64::MAX)),
            opt(planned.from_seq),
            opt(planned.to_seq),
            SqlCell::Int(crate::commands::effect::BACKFILL_BATCH as i64),
        ]])
    }

    /// The files and folders the stream's worktree's filter keeps: in
    /// `parent` alone when the query names one, else the whole tree.
    async fn workspace_files(
        &self,
        stream: Option<StreamId>,
        parent: Option<String>,
    ) -> Result<Vec<Vec<SqlCell>>, DomainError> {
        let stream = self.stream(stream).await?;
        let generated = self
            .config
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .generated
            .clone();
        let paths = self
            .workspace_files
            .list_paths(Some(&stream.id.to_string()), &generated, parent)
            .await
            .map_err(|e| DomainError::Invalid(e.to_string()))?;
        let id = SqlCell::Int(stream.id.value());
        Ok(paths
            .into_iter()
            .map(|p| {
                let (parent, name) = match p.path.rsplit_once('/') {
                    Some((parent, name)) => (parent.to_string(), name.to_string()),
                    None => (String::new(), p.path.clone()),
                };
                vec![
                    id.clone(),
                    text(p.path),
                    text(parent),
                    text(name),
                    text(if p.is_dir { "dir" } else { "file" }),
                    opt(p.status.map(|s| s.as_str())),
                ]
            })
            .collect())
    }
}
