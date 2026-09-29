//! Cores for the `events` command module: the event log's dead-letter
//! queue (`.context/data-model.md` "event_log").
//!
//! A consumer that fails on an event parks it here with the error; the
//! pump moves on. These are the person's three moves: see what's parked,
//! run it again (after fixing the cause), or give up on it visibly.

use oxplow_app::Services;
use oxplow_db::DeadLetter;

use crate::error::IpcError;

/// Parked events. `pending` only by default; `all` includes `retried`
/// and `discarded` letters.
pub async fn list_dead_letters(
    svc: &Services,
    all: Option<bool>,
) -> Result<Vec<DeadLetter>, IpcError> {
    Ok(svc
        .event_pump
        .list_dead_letters(all.unwrap_or(false))
        .await?)
}

/// Run the parked event through its consumer again, now. The letter
/// becomes `retried` on success; on failure it stays `pending` with the
/// new error and one more attempt. Returns the letter's new state.
pub async fn retry_dead_letter(svc: &Services, id: i64) -> Result<DeadLetter, IpcError> {
    Ok(svc.event_pump.retry_dead_letter(id).await?)
}

/// Give up on the parked event. It stays visible as `discarded`.
pub async fn discard_dead_letter(svc: &Services, id: i64) -> Result<DeadLetter, IpcError> {
    Ok(svc.event_pump.discard_dead_letter(id).await?)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn list_dead_letters_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch("list_dead_letters", serde_json::json!({}), &svc)
            .await
            .unwrap();
        assert_eq!(out, serde_json::json!([]));
    }

    #[tokio::test]
    async fn retry_and_discard_report_not_found_for_an_unknown_letter() {
        let (svc, _dir) = crate::test_support::services();
        for name in ["retry_dead_letter", "discard_dead_letter"] {
            let err = crate::dispatch(name, serde_json::json!({"id": 424242}), &svc)
                .await
                .unwrap_err();
            assert!(err.to_string().contains("not found"), "{name}: {err}");
        }
    }
}
