//! Terminal commands (`.context/commands.md`): a stream's terminals are
//! rows (`terminal`, `v_terminal`), each a shell in the stream's worktree
//! named by its URI. Opening and renaming are `Tx` over the row; closing
//! reaches the PTY the daemon keeps for it, so it is `External` — the shell
//! ends, then the row goes. An agent opens and renames only in its own
//! stream; closing is a person's (the shell may hold their work).

use std::sync::Arc;

use oxplow_db::terminal_store::{delete_tx, open_tx, rename_tx, terminal_tx};
use oxplow_db::{Database, SqliteTerminalStore};
use oxplow_domain::events::schema::{
    TerminalClosed, TerminalClosedV1, TerminalOpened, TerminalOpenedV1, TerminalRenamed,
    TerminalRenamedV1,
};
use oxplow_domain::events::Envelope;
use oxplow_domain::uri::build::terminal_uri;
use oxplow_domain::{CommandCall, CommandError, Confirm, StreamId, TerminalId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::ops::Op;
use super::thread::agent_scope;
use super::util::{bare_id, invalid, parse, schema, sql};
use super::{Handler, HandlerOutput, Invocation, TxCtx};
use crate::terminal_sessions::{terminal_key, TerminalSessionRegistry};

pub const OPEN: &str = "oxplow.terminal.open";
pub const RENAME: &str = "oxplow.terminal.rename";
pub const CLOSE: &str = "oxplow.terminal.close";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenInput {
    /// The stream (`str2`) whose worktree the shell runs in.
    pub stream: String,
    /// Its title; left out, the next "Terminal N".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameInput {
    /// The terminal (`trm3`).
    pub terminal: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloseInput {
    /// The terminal (`trm3`).
    pub terminal: String,
}

fn terminal_id(value: &str) -> Result<TerminalId, CommandError> {
    bare_id(value, oxplow_domain::uri::resources::TERMINAL, "/terminal")
}

/// An agent works only in its own stream.
fn own_stream(ctx: &TxCtx<'_>, stream: StreamId) -> Result<(), CommandError> {
    if let Some((_, own)) = agent_scope(ctx)? {
        if own != stream {
            return Err(CommandError::Denied {
                reason: format!("an agent's terminals are its own stream's (`{own}`)"),
            });
        }
    }
    Ok(())
}

fn event<T: oxplow_domain::EventType>(
    ctx: &TxCtx<'_>,
    stream: StreamId,
    terminal: TerminalId,
    payload: &T::Payload,
) -> Envelope {
    let mut event = Envelope::typed::<T>(ctx.actor().source(), payload);
    event.anchors.stream_id = Some(stream);
    event.with_subject([terminal_uri(terminal)])
}

/// `terminal.open { stream, title? }`: a shell at the end of the stream's
/// terminals; answers `{ id, uri }`. Undone by closing it.
pub fn open_op() -> Op {
    Op::new(
        "terminals.write",
        "open",
        schema::<OpenInput>(),
        false,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: OpenInput = parse(input)?;
            let stream: StreamId = bare_id(
                &input.stream,
                oxplow_domain::uri::resources::STREAM,
                "/stream",
            )?;
            own_stream(ctx, stream)?;
            oxplow_db::stream_store::get_tx(ctx.conn, stream)
                .map_err(sql)?
                .filter(|s| s.archived_at.is_none())
                .ok_or_else(|| invalid("/stream", format!("no stream `{stream}`")))?;
            let made = open_tx(ctx.conn, stream, input.title.as_deref())?;
            Ok(HandlerOutput {
                result: json!({ "id": made.id.to_string(), "uri": terminal_uri(made.id), "title": made.title }),
                events: vec![event::<TerminalOpened>(
                    ctx,
                    stream,
                    made.id,
                    &TerminalOpenedV1 {
                        stream: stream.to_string(),
                        terminal: made.id.to_string(),
                        title: made.title.clone(),
                    },
                )],
                ..HandlerOutput::default()
            })
        })),
    )
}

/// `terminal.rename { terminal, title }`; undone by renaming it back.
pub fn rename_op() -> Op {
    Op::new(
        "terminals.write",
        "rename",
        schema::<RenameInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: RenameInput = parse(input)?;
            let id = terminal_id(&input.terminal)?;
            let title = input.title.trim().to_string();
            if title.is_empty() {
                return Err(invalid("/title", "a terminal's title can't be blank"));
            }
            let before = terminal_tx(ctx.conn, id)?
                .ok_or_else(|| invalid("/terminal", format!("no terminal `{id}`")))?;
            own_stream(ctx, before.stream_id)?;
            rename_tx(ctx.conn, id, &title)?;
            Ok(HandlerOutput {
                result: json!({ "id": id.to_string(), "uri": terminal_uri(id), "title": title }),
                inverse: Some(CommandCall {
                    name: RENAME.into(),
                    input: json!({ "terminal": input.terminal, "title": before.title }),
                }),
                events: vec![event::<TerminalRenamed>(
                    ctx,
                    before.stream_id,
                    id,
                    &TerminalRenamedV1 {
                        stream: before.stream_id.to_string(),
                        terminal: id.to_string(),
                        title,
                    },
                )],
                ..HandlerOutput::default()
            })
        })),
    )
}

/// `terminal.close { terminal }`: the shell ends and the row goes. The last
/// terminal may close — the page offers New. A person's: a shell may hold
/// work, so an agent never closes one (and a person confirms).
pub fn close_op(
    db: Database,
    terminals: Arc<SqliteTerminalStore>,
    sessions: TerminalSessionRegistry,
) -> Op {
    Op::new(
        "terminals.write",
        "close",
        schema::<CloseInput>(),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input| {
            let (db, terminals, sessions) = (db.clone(), terminals.clone(), sessions.clone());
            Box::pin(async move {
                if !invocation.actor().may_confirm() {
                    return Err(CommandError::Denied {
                        reason: "a person closes a terminal: its shell may hold work".into(),
                    });
                }
                let input: CloseInput = parse(input)?;
                let id = terminal_id(&input.terminal)?;
                let row = terminals
                    .get(id)
                    .await?
                    .ok_or_else(|| invalid("/terminal", format!("no terminal `{id}`")))?;
                // The shell first: a row with no shell is a terminal the page
                // offers to start again; a shell with no row is invisible.
                sessions.close_key(&terminal_key(id)).await;
                db.transaction(move |tx| delete_tx(tx, id)).await?;
                let mut closed = Envelope::typed::<TerminalClosed>(
                    invocation.actor().source(),
                    &TerminalClosedV1 {
                        stream: row.stream_id.to_string(),
                        terminal: id.to_string(),
                        title: row.title.clone(),
                    },
                );
                closed.anchors.stream_id = Some(row.stream_id);
                let closed = closed.with_subject([terminal_uri(id)]);
                Ok(HandlerOutput {
                    result: json!({ "id": id.to_string(), "closed": true }),
                    events: vec![closed],
                    ..HandlerOutput::default()
                })
            })
        })),
    )
    .confirm_at_least(Confirm::Destructive)
}

pub fn ops(
    db: Database,
    terminals: Arc<SqliteTerminalStore>,
    sessions: TerminalSessionRegistry,
) -> Vec<Op> {
    vec![open_op(), rename_op(), close_op(db, terminals, sessions)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{services_with_effort, EffortFixture};
    use oxplow_domain::Actor;

    fn agent(fx: &EffortFixture) -> Actor {
        Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        }
    }

    async fn stream_id(fx: &EffortFixture) -> String {
        use oxplow_domain::stores::StreamStore as _;
        fx.svc
            .stream_store
            .primary()
            .await
            .unwrap()
            .unwrap()
            .id
            .to_string()
    }

    async fn run(
        fx: &EffortFixture,
        actor: &Actor,
        name: &str,
        input: serde_json::Value,
        confirmed: bool,
    ) -> Result<oxplow_domain::CommandOutcome, CommandError> {
        fx.svc.commands.run(actor, name, input, confirmed).await
    }

    async fn titles(fx: &EffortFixture) -> Vec<String> {
        let out = fx
            .svc
            .sql
            .query_sql(
                "SELECT title FROM v_terminal ORDER BY position",
                vec![],
                None,
            )
            .await
            .unwrap();
        serde_json::to_value(&out.rows)
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r[0].as_str().unwrap().to_string())
            .collect()
    }

    /// Opening numbers the titles past the highest and the count, answers
    /// the row's id and URI, and logs `terminal.opened`; a given title is
    /// kept; renaming changes the title (and undoes).
    #[tokio::test]
    async fn open_numbers_rename_renames() {
        let fx = services_with_effort().await;
        let stream = stream_id(&fx).await;
        let first = run(&fx, &Actor::Human, OPEN, json!({ "stream": stream }), false)
            .await
            .unwrap();
        assert_eq!(first.result["title"], "Terminal 1");
        let id = first.result["id"].as_str().unwrap().to_string();
        assert_eq!(
            first.result["uri"],
            format!("oxplow:oxplow/terminal?id={id}")
        );
        run(&fx, &Actor::Human, OPEN, json!({ "stream": stream }), false)
            .await
            .unwrap();
        run(
            &fx,
            &Actor::Human,
            OPEN,
            json!({ "stream": stream, "title": "build" }),
            false,
        )
        .await
        .unwrap();
        assert_eq!(titles(&fx).await, ["Terminal 1", "Terminal 2", "build"]);
        let renamed = run(
            &fx,
            &Actor::Human,
            RENAME,
            json!({ "terminal": id, "title": "server" }),
            false,
        )
        .await
        .unwrap();
        assert_eq!(titles(&fx).await[0], "server");
        assert!(renamed.inverse.is_some());
        // The next title is past the highest numbered one, not the renamed.
        let next = run(&fx, &Actor::Human, OPEN, json!({ "stream": stream }), false)
            .await
            .unwrap();
        assert_eq!(next.result["title"], "Terminal 4");
        let logged: Vec<String> = fx
            .svc
            .event_log_store
            .read_after(0, 1000)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.envelope.event_type)
            .filter(|t| t.starts_with("terminal."))
            .collect();
        assert_eq!(logged.iter().filter(|t| *t == "terminal.opened").count(), 4);
        assert_eq!(
            logged.iter().filter(|t| *t == "terminal.renamed").count(),
            1
        );
        // A blank title and an unknown stream are refused at their fields.
        let blank = run(
            &fx,
            &Actor::Human,
            RENAME,
            json!({ "terminal": id, "title": "  " }),
            false,
        )
        .await
        .unwrap_err();
        assert!(blank.to_string().contains("blank"), "{blank}");
        let missing = run(
            &fx,
            &Actor::Human,
            OPEN,
            json!({ "stream": "str9999" }),
            false,
        )
        .await
        .unwrap_err();
        assert!(missing.to_string().contains("str9999"), "{missing}");
    }

    /// Closing ends the shell the registry keeps under the terminal's URI
    /// and deletes the row; an agent can't close one.
    #[tokio::test]
    async fn close_ends_the_shell_and_deletes_the_row() {
        let fx = services_with_effort().await;
        let stream = stream_id(&fx).await;
        let opened = run(&fx, &Actor::Human, OPEN, json!({ "stream": stream }), false)
            .await
            .unwrap();
        let id = opened.result["id"].as_str().unwrap().to_string();
        let tid: TerminalId = id.parse().unwrap();
        let key = terminal_key(tid);
        fx.svc
            .terminal_sessions
            .attach_or_create(key.clone(), 80, 24, |c, r| {
                crate::terminal_sessions::SpawnRequest {
                    command: "sh".into(),
                    args: vec!["-c".into(), "sleep 30".into()],
                    cwd: std::env::temp_dir(),
                    env: Vec::new(),
                    env_remove: Vec::new(),
                    cols: c,
                    rows: r,
                }
            })
            .await
            .unwrap();
        assert!(fx
            .svc
            .terminal_sessions
            .session_id_for_key(&key)
            .await
            .is_some());

        // An agent never closes one.
        let asked = run(&fx, &agent(&fx), CLOSE, json!({ "terminal": id }), true)
            .await
            .unwrap_err();
        assert!(matches!(asked, CommandError::Denied { .. }), "{asked:?}");
        assert_eq!(titles(&fx).await.len(), 1);
        assert!(fx
            .svc
            .terminal_sessions
            .session_id_for_key(&key)
            .await
            .is_some());

        let closed = run(&fx, &Actor::Human, CLOSE, json!({ "terminal": id }), true)
            .await
            .unwrap();
        assert_eq!(closed.result["closed"], true);
        assert!(fx
            .svc
            .terminal_sessions
            .session_id_for_key(&key)
            .await
            .is_none());
        assert!(titles(&fx).await.is_empty());
        // The last one may close; closing it again is refused at its field.
        let again = run(&fx, &Actor::Human, CLOSE, json!({ "terminal": id }), true)
            .await
            .unwrap_err();
        assert!(again.to_string().contains(&id), "{again}");
    }
}
