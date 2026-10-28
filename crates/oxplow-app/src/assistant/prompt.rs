//! What the Assistant is told (its system prompt) and what "Continue in
//! session" hands over (a condensed transcript). Neither reads the
//! thread's context basket, its sessions' context or its handoffs.

use crate::chat::model::ToolStatus;
use crate::chat::transcript::{ItemBody, TranscriptItem};

use super::AssistantContext;

/// Where the session works, as the system prompt names it.
pub struct Place {
    pub stream: String,
    pub stream_title: String,
    pub branch: String,
    pub worktree: String,
    pub thread: Option<(String, String)>,
}

/// The system prompt: who it is, how its tools work, where it works and
/// what it was opened on.
pub fn system(place: &Place, context: Option<&AssistantContext>, allow_changes: bool) -> String {
    let mut out = String::from(
        "You are oxplow's Assistant: a short-lived helper in a panel of oxplow, a desktop \
         app for software work with coding agents. Answer the person's questions and do small, \
         scoped tasks for them. Be brief and concrete; keep names, numbers and paths exact.\n\n\
         Your tools are oxplow's commands. Read before you answer: query oxplow's datasets with \
         `oxplow_sql_query` (`v_dataset` lists them), read anything a URI names with \
         `oxplow_records_read`, and search with `oxplow_search_find`. ",
    );
    if allow_changes {
        out.push_str(
            "To change something, find the command with `oxplow_find_command` and run it \
             with `oxplow_run_command`: the person is asked first. If they decline, say what \
             you would have done and stop. Change only what was asked.\n\n",
        );
    } else {
        out.push_str(
            "You can only read now: the person hasn't allowed changes. Say what you would \
             change and that they can turn on \"Allow changes\".\n\n",
        );
    }
    out.push_str(&format!(
        "You work in stream `{}` ({}), branch `{}`, worktree `{}`. A read that takes a \
         `stream` reads the person's own when it names none; name `{}` to read this one.\n",
        place.stream, place.stream_title, place.branch, place.worktree, place.stream
    ));
    if let Some((id, title)) = &place.thread {
        out.push_str(&format!("The person is in thread `{id}` ({title}).\n"));
    }
    match context {
        Some(AssistantContext::Page { uri, label }) => {
            out.push_str(&format!(
                "\nThey opened you on a page: {} (`{uri}`). Read it when the question is \
                 about it.\n",
                label.as_deref().unwrap_or("this page")
            ));
        }
        Some(AssistantContext::Selection { uri, text, label }) => {
            out.push_str("\nThey opened you on a selection");
            if let Some(label) = label {
                out.push_str(&format!(" from {label}"));
            }
            if let Some(uri) = uri {
                out.push_str(&format!(" (`{uri}`)"));
            }
            out.push_str(&format!(":\n\n```\n{text}\n```\n"));
        }
        None => {}
    }
    out
}

/// What "Continue in session" hands over: the opening context's ref, the
/// person's prompts, the Assistant's final answers and one line per tool
/// call.
pub fn handoff_body(context: Option<&AssistantContext>, items: &[TranscriptItem]) -> String {
    let mut out = String::from("Continued from the Assistant.\n");
    match context {
        Some(AssistantContext::Page { uri, label }) => {
            out.push_str(&format!(
                "\nAbout: {} (`{uri}`)\n",
                label.as_deref().unwrap_or("a page")
            ));
        }
        Some(AssistantContext::Selection { uri, label, .. }) => {
            let what = label.as_deref().unwrap_or("a selection");
            match uri {
                Some(uri) => out.push_str(&format!("\nAbout: {what} (`{uri}`)\n")),
                None => out.push_str(&format!("\nAbout: {what}\n")),
            }
        }
        None => {}
    }
    // The agent's last message before the person spoke again is that
    // turn's answer; earlier ones were progress.
    let mut answer: Option<&str> = None;
    let flush = |out: &mut String, answer: &mut Option<&str>| {
        if let Some(text) = answer.take() {
            out.push_str(&format!("\n**Assistant:** {}\n", text.trim()));
        }
    };
    for item in items {
        match &item.body {
            ItemBody::User { text, .. } => {
                flush(&mut out, &mut answer);
                out.push_str(&format!("\n**Person:** {}\n", text.trim()));
            }
            ItemBody::Agent { text } if !text.trim().is_empty() => answer = Some(text),
            ItemBody::Tool { call } => {
                let name = call.name.as_deref().unwrap_or(&call.title);
                let how = match call.status {
                    ToolStatus::Completed => "done",
                    ToolStatus::Failed => "failed or declined",
                    ToolStatus::Pending | ToolStatus::InProgress => "stopped",
                };
                out.push_str(&format!("- `{name}`: {how}\n"));
            }
            _ => {}
        }
    }
    flush(&mut out, &mut answer);
    out
}
