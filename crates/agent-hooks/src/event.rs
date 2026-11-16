//! The canonical completion event shared by every agent adapter.
//!
//! Each agent CLI emits its hook payload in its own native shape (or, for
//! plugin-based agents, in the shape the installed plugin sends). [`extract`]
//! maps any of those wire shapes into a single [`NormalizedEvent`] so the
//! rest of EMA works against one schema instead of twelve.
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Terminal state of the agent turn that produced the event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    Completed,
    Failed,
    Interrupted,
}

/// One normalized completion event, regardless of which agent produced it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedEvent {
    /// Registry name of the producing agent, e.g. "codex".
    pub agent: &'static str,
    /// Native wire event type, e.g. "agent-turn-complete", "Stop", "agent_end".
    pub event_type: String,
    /// Native session/thread identifier ("" when the agent does not expose one).
    pub session_id: String,
    /// Stable per-turn identity used for de-duplication ("");
    pub event_id: String,
    /// Working directory of the agent run ("" when unknown).
    pub cwd: String,
    /// Native session title when the agent provides one.
    pub title: String,
    /// The user prompt that started the turn.
    pub prompt: String,
    /// The final assistant response.
    pub response: String,
    /// Recent conversation context as (role, text) pairs, oldest last is NOT
    /// guaranteed; renderers take the most recent two.
    pub context_messages: Vec<(String, String)>,
    pub status: EventStatus,
    pub prompt_truncated: bool,
    pub response_truncated: bool,
    /// The original wire payload. Report renderers may read agent-specific
    /// extras (e.g. codex `turn-id`, claude `transcript_path`) from here.
    pub raw: Value,
}

fn field<'a>(v: &'a Value, keys: &[&str]) -> &'a str {
    keys.iter()
        .filter_map(|key| v.get(*key).and_then(Value::as_str))
        .find(|text| !text.trim().is_empty())
        .unwrap_or("")
}

/// Extract text from a native content value: either a string or an array of
/// parts, keeping only `text`-typed parts.
fn content(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        return s.to_owned();
    }
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| {
            if let Some(s) = part.as_str() {
                Some(s)
            } else if part["type"] == "text" {
                part["text"].as_str()
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Map one native wire payload into the canonical schema.
/// `agent` must be the registry name of the producer.
pub fn extract(agent: &'static str, wire_event_type: &str, v: &Value) -> NormalizedEvent {
    let session_id = field(v, &["thread-id", "session_id", "sessionID"]).to_owned();
    let event_id = field(v, &["turn-id", "turn_id", "event_id"]).to_owned();
    let cwd = field(v, &["cwd"]).to_owned();
    let title = field(v, &["session_title", "thread_title", "title"]).to_owned();

    let mut prompt = field(v, &["prompt", "user_prompt", "user_message", "question"]).to_owned();
    if prompt.trim().is_empty() {
        prompt = content(&v["prompt"]);
    }
    // codex's `input-messages` is the argv echo of the notify call, not the user's
    // instruction — it must not leak into reports as the prompt. Leave it empty.
    let response = field(
        v,
        &["last-assistant-message", "last_assistant_message", "prompt_response", "response"],
    )
    .to_owned();

    let context_messages = v["context_messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|message| {
            let role = message["role"].as_str()?;
            let text = message["text"].as_str()?;
            if matches!(role, "user" | "assistant") && !text.trim().is_empty() {
                Some((role.to_owned(), text.to_owned()))
            } else {
                None
            }
        })
        .collect();

    // Kimi signals failure through the event type; other agents carry explicit flags.
    let failed = (agent == "kimi" && wire_event_type == "StopFailure")
        || v["failed"].as_bool() == Some(true)
        || v["completed"].as_bool() == Some(false);
    let interrupted = v["interrupted"].as_bool() == Some(true);
    let status = if interrupted {
        EventStatus::Interrupted
    } else if failed {
        EventStatus::Failed
    } else {
        EventStatus::Completed
    };

    NormalizedEvent {
        agent,
        event_type: wire_event_type.to_owned(),
        session_id,
        event_id,
        cwd,
        title,
        prompt,
        response,
        context_messages,
        status,
        prompt_truncated: v["prompt_truncated"].as_bool() == Some(true),
        response_truncated: v["response_truncated"].as_bool() == Some(true),
        raw: v.clone(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn codex_native_notify_does_not_leak_input_messages() {
        let raw = serde_json::json!({
            "type": "agent-turn-complete", "thread-id": "native-session", "turn-id": "native-turn",
            "cwd": "/tmp/mail-qa", "client": "exec",
            "input-messages": ["核对邮件内容", "保留中文指令"],
            "last-assistant-message": "内容已核对"
        });
        let event = super::extract("codex", "agent-turn-complete", &raw);
        assert_eq!(event.prompt, "", "input-messages is the argv echo and must not leak");
        assert_eq!(event.response, "内容已核对");
        assert_eq!(event.session_id, "native-session");
    }
}
