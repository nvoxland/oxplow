//! Stored datasets made on demand (`.context/semantic-layer.md` "Made on
//! demand"): a query that names one thing of such a dataset by its keys has
//! that thing stored before it runs, when the store lacks it. A commit the
//! index never reached (`v_commit`, `v_commit_file` by `sha`) is indexed;
//! a change (`v_change` by `kind` and `target`) is created and its
//! analysis started. So a page, or an agent's query, reads any commit git
//! knows and any change it names, through the datasets it already reads.
//!
//! The stored-dataset twin of a live dataset's `pushdown`: the same certain
//! `<column> = <value>` conditions (`sql_tokens::text_filters`). A query
//! that names the thing any other way (a join, an `OR`, a subquery) asks
//! for nothing and reads what is stored.
//!
//! What is made is what the indexer and `ensure_change` store anyway: no
//! row here is of a kind the store doesn't already hold. A failure to make
//! it is logged and the query reads what is stored: a read never fails
//! because a side computation did.

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

use oxplow_db::SqlParams;
use oxplow_domain::DomainError;

use crate::commands::util::ServicesSlot;

/// The datasets made on demand (`v_<name>`) and the columns a query pins,
/// every one of them, to ask for one thing.
pub fn keys(dataset: &str) -> &'static [&'static str] {
    match dataset {
        "commit" | "commit_file" => &["sha"],
        "change" => &["kind", "target"],
        _ => &[],
    }
}

const DATASETS: &[&str] = &["commit", "commit_file", "change"];

/// One thing a query asks for: its dataset and each key's value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub dataset: &'static str,
    pub keys: Vec<(String, String)>,
}

impl Request {
    fn key(&self, name: &str) -> Option<&str> {
        self.keys
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// What `sql`, run with `params`, asks to be made: each dataset of [`keys`]
/// it pins by all of that dataset's keys.
pub fn requests(sql: &str, params: &SqlParams) -> Result<Vec<Request>, DomainError> {
    let mut out = Vec::new();
    for dataset in DATASETS {
        let wanted = keys(dataset);
        let found =
            oxplow_db::sql_tokens::text_filters(sql, &format!("v_{dataset}"), wanted, params)?;
        let mut pinned = Vec::new();
        for key in wanted {
            match found.iter().find(|(c, _)| c.eq_ignore_ascii_case(key)) {
                Some((_, v)) => pinned.push((key.to_string(), v.clone())),
                None => break,
            }
        }
        if pinned.len() == wanted.len() {
            out.push(Request {
                dataset,
                keys: pinned,
            });
        }
    }
    Ok(out)
}

/// The gateway's handle on what makes a request: the services, once there
/// are some. Shared by every clone of the gateway, bound once; a gateway
/// with none bound (a check, a test of the store alone) makes nothing.
#[derive(Default)]
pub struct OnDemand {
    services: OnceLock<ServicesSlot>,
}

impl OnDemand {
    pub fn bind(&self, services: ServicesSlot) {
        let _ = self.services.set(services);
    }

    /// Make what `request` asks for, for `stream` (a row id; none: the
    /// primary checkout). Boxed: making a change reads through the gateway.
    pub fn ensure<'a>(
        &'a self,
        request: &'a Request,
        stream: Option<i64>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let Some(svc) = self.services.get().and_then(|s| s.get().ok()) else {
                return;
            };
            let made = match request.dataset {
                "commit" | "commit_file" => match request.key("sha") {
                    Some(sha) => crate::commit_indexer::ensure_commit(&svc, stream, sha)
                        .await
                        .map(|_| ()),
                    None => Ok(()),
                },
                "change" => match change_target(request, stream) {
                    Some(target) => crate::change_analysis::request(&svc, target).await,
                    None => Ok(()),
                },
                _ => Ok(()),
            };
            if let Err(e) = made {
                tracing::warn!(?e, ?request, "a query's on-demand read failed");
            }
        })
    }
}

/// The change a `v_change` request names, run for `stream`: a `commit` by
/// its sha, the `working` tree (target `''`), an `effort` or a `turn` by
/// its row id, a `range` by its two sides (`<base>..<head>`, the base
/// empty for "from nothing"). Anything else names none.
fn change_target(
    request: &Request,
    stream: Option<i64>,
) -> Option<crate::change_analysis::ChangeTarget> {
    use crate::change_analysis::ChangeTarget;
    let stream_id = stream.map(|s| oxplow_domain::StreamId::new(s).to_string());
    let target = request.key("target")?;
    Some(match request.key("kind")? {
        "commit" if !target.is_empty() => ChangeTarget::Commit {
            sha: target.to_string(),
            stream_id,
        },
        "working" if target.is_empty() => ChangeTarget::Working {
            stream_id: stream_id?,
        },
        "effort" => ChangeTarget::Effort {
            effort_id: oxplow_domain::EffortId::new(target.parse().ok()?).to_string(),
        },
        "turn" => ChangeTarget::Turn {
            turn_id: oxplow_domain::AgentTurnId::new(target.parse().ok()?).to_string(),
        },
        "range" => {
            let (base, head) = target.split_once("..")?;
            if head.is_empty() {
                return None;
            }
            ChangeTarget::Range {
                stream_id,
                base: (!base.is_empty()).then(|| base.to_string()),
                head: head.to_string(),
            }
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::SqlCell;

    fn named(pairs: &[(&str, &str)]) -> SqlParams {
        SqlParams::Named(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), SqlCell::Text(v.to_string())))
                .collect(),
        )
    }

    fn request(dataset: &'static str, keys: &[(&str, &str)]) -> Request {
        Request {
            dataset,
            keys: keys
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    /// A query that pins a dataset by all its keys asks for that one thing;
    /// by some of them, or any way but a plain condition, it asks for
    /// nothing.
    #[test]
    fn a_query_that_names_one_thing_by_its_keys_asks_for_it() {
        let p = named(&[("sha", "abc")]);
        assert_eq!(
            requests(
                "SELECT id AS change_id FROM v_change WHERE kind = 'commit' AND target = :sha",
                &p
            )
            .unwrap(),
            vec![request("change", &[("kind", "commit"), ("target", "abc")])]
        );
        assert_eq!(
            requests(
                "SELECT subject FROM v_commit WHERE sha = ?1",
                &SqlParams::Positional(vec![SqlCell::Text("def".into())])
            )
            .unwrap(),
            vec![request("commit", &[("sha", "def")])]
        );
        assert_eq!(
            requests("SELECT path FROM v_commit_file f WHERE f.sha = :sha", &p).unwrap(),
            vec![request("commit_file", &[("sha", "abc")])]
        );
        // A range's target is built in SQL from its two sides.
        let sides = named(&[("start", "snap:1"), ("end", "working")]);
        assert_eq!(
            requests(
                "SELECT id FROM v_change WHERE kind = 'range' AND target = :start || '..' || :end",
                &sides
            )
            .unwrap(),
            vec![request(
                "change",
                &[("kind", "range"), ("target", "snap:1..working")]
            )]
        );
        assert_eq!(
            requests(
                "SELECT id FROM v_change c WHERE kind = 'range' AND target = :start || '..' || c.head_revision",
                &sides
            )
            .unwrap(),
            vec![]
        );
        // The one read, inside a subquery, is narrowed by its own `SELECT`.
        assert_eq!(
            requests(
                "SELECT 1 FROM v_stream WHERE id IN (SELECT 1 FROM v_commit WHERE sha = :sha)",
                &p
            )
            .unwrap(),
            vec![request("commit", &[("sha", "abc")])]
        );
        for sql in [
            "SELECT id FROM v_change WHERE kind = 'commit'",
            "SELECT id FROM v_change WHERE kind = 'commit' OR target = :sha",
            "SELECT c.subject FROM v_commit c JOIN v_commit_file f ON f.sha = c.sha WHERE c.sha = :sha",
            "SELECT subject FROM v_commit WHERE sha = :missing",
        ] {
            assert_eq!(requests(sql, &p).unwrap(), vec![], "{sql}");
        }
    }

    /// `v_change`'s keys name a change: a commit by sha, the working tree
    /// of the run's stream, an effort or a turn by row id.
    #[test]
    fn a_changes_keys_name_its_target() {
        use crate::change_analysis::ChangeTarget;
        assert_eq!(
            change_target(
                &request("change", &[("kind", "commit"), ("target", "abc")]),
                Some(1)
            ),
            Some(ChangeTarget::Commit {
                sha: "abc".into(),
                stream_id: Some("str1".into())
            })
        );
        assert_eq!(
            change_target(
                &request("change", &[("kind", "working"), ("target", "")]),
                Some(2)
            ),
            Some(ChangeTarget::Working {
                stream_id: "str2".into()
            })
        );
        assert_eq!(
            change_target(
                &request("change", &[("kind", "effort"), ("target", "7")]),
                None
            ),
            Some(ChangeTarget::Effort {
                effort_id: "eff7".into()
            })
        );
        // A range: `<base>..<head>`, the base empty for "from nothing".
        assert_eq!(
            change_target(
                &request(
                    "change",
                    &[("kind", "range"), ("target", "snap:1..working")]
                ),
                Some(1)
            ),
            Some(ChangeTarget::Range {
                stream_id: Some("str1".into()),
                base: Some("snap:1".into()),
                head: "working".into()
            })
        );
        assert_eq!(
            change_target(
                &request("change", &[("kind", "range"), ("target", "..git:abc")]),
                None
            ),
            Some(ChangeTarget::Range {
                stream_id: None,
                base: None,
                head: "git:abc".into()
            })
        );
        for (kind, target, stream) in [
            ("working", "", None),
            ("working", "x", Some(1)),
            ("commit", "", Some(1)),
            ("effort", "x", None),
            ("other", "1", Some(1)),
            ("range", "snap:1..", Some(1)),
            ("range", "snap:1", Some(1)),
        ] {
            assert_eq!(
                change_target(
                    &request("change", &[("kind", kind), ("target", target)]),
                    stream
                ),
                None,
                "{kind} {target}"
            );
        }
    }
}
