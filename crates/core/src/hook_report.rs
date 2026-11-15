//! Human-readable reports for local Agent completion events.
use serde_json::Value;
use std::{collections::VecDeque, fs::{self, File}, io::{BufRead, BufReader, Seek, SeekFrom}, path::{Path, PathBuf}};

fn field<'a>(v: &'a Value, keys: &[&str]) -> &'a str {
    keys.iter().filter_map(|key| v.get(*key).and_then(Value::as_str))
        .find(|text| !text.trim().is_empty()).unwrap_or("")
}
fn excerpt(text: &str, limit: usize, zh: bool) -> String {
    let text: String = text.chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').collect();
    let text = text.trim();
    let mut chars = text.chars();
    let prefix: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        format!("{}{}", prefix.trim_end(), if zh { "\n…（内容已截断）" } else { "\n… (truncated)" })
    } else { prefix }
}
fn single_line(text: &str, limit: usize) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").chars().filter(|c| !c.is_control()).take(limit).collect()
}
fn content(v: &Value) -> String {
    if let Some(s) = v.as_str() { return s.to_owned(); }
    v.as_array().into_iter().flatten().filter_map(|part| {
        if let Some(s) = part.as_str() { Some(s) }
        else if part["type"] == "text" { part["text"].as_str() }
        else { None }
    }).collect::<Vec<_>>().join("\n")
}
fn question(v: &Value) -> String {
    let input = content(&v["input-messages"]);
    if !input.trim().is_empty() { return input; }
    let direct = field(v, &["prompt", "user_prompt", "user_message", "question"]);
    if !direct.is_empty() { return direct.to_owned(); }
    content(&v["prompt"])
}
fn home() -> Option<PathBuf> { std::env::var_os("HOME").map(PathBuf::from) }
fn codex_root() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME").map(PathBuf::from).or_else(|| home().map(|h| h.join(".codex")))
}
fn codex_title(session: &str) -> Option<String> {
    if session.is_empty() { return None; }
    let root = codex_root()?;
    let lines = BufReader::new(File::open(root.join("session_index.jsonl")).ok()?).lines();
    let mut title = None;
    for line in lines.map_while(Result::ok) {
        let Ok(entry) = serde_json::from_str::<Value>(&line) else { continue; };
        if entry["id"].as_str() == Some(session) {
            if let Some(name) = entry["thread_name"].as_str().filter(|s| !s.trim().is_empty()) {
                title = Some(name.to_owned());
            }
        }
    }
    title
}

fn codex_context(session: &str, turn: &str) -> Vec<(String, String)> {
    use rusqlite::{Connection, OpenFlags};
    let Some(root) = codex_root() else { return Vec::new(); };
    if session.is_empty() || turn.is_empty() { return Vec::new(); }
    let mut states: Vec<_> = fs::read_dir(&root).into_iter().flatten().flatten().filter_map(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        let version = name.strip_prefix("state_")?.strip_suffix(".sqlite")?.parse::<u32>().ok()?;
        Some((version, entry.path()))
    }).collect();
    states.sort_by(|a, b| b.0.cmp(&a.0));
    let mut rollout = None;
    for (_, db_path) in states {
        let Ok(db) = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX) else { continue; };
        let _ = db.busy_timeout(std::time::Duration::from_millis(50));
        if let Ok(path) = db.query_row("SELECT rollout_path FROM threads WHERE id=?1", [session], |row| row.get::<_, String>(0)) {
            rollout = Some(PathBuf::from(path)); break;
        }
    }
    let Some(path) = rollout.and_then(|p| p.canonicalize().ok()) else { return Vec::new(); };
    if !["sessions", "archived_sessions"].iter().any(|name| root.join(name).canonicalize().map(|base| path.starts_with(base)).unwrap_or(false)) { return Vec::new(); }
    let Ok(mut file) = File::open(path) else { return Vec::new(); };
    let Ok(meta) = file.metadata() else { return Vec::new(); };
    let offset = meta.len().saturating_sub(2 * 1024 * 1024);
    if file.seek(SeekFrom::Start(offset)).is_err() { return Vec::new(); }
    let mut reader = BufReader::new(file);
    if offset > 0 { let _ = reader.read_line(&mut String::new()); }
    let mut recent = VecDeque::new();
    let mut matched_turn = false;
    for line in reader.lines().map_while(Result::ok) {
        let Ok(record) = serde_json::from_str::<Value>(&line) else { continue; };
        let event = &record["payload"];
        if record["type"] != "event_msg" || event["type"] != "item_completed" { continue; }
        if event["turn_id"].as_str() == Some(turn) { matched_turn = true; break; }
        let item = &event["item"];
        let role = match item["type"].as_str() {
            Some("UserMessage") => "user",
            Some("AgentMessage") if item["phase"] == "final_answer" => "assistant",
            _ => continue,
        };
        let text = item["content"].as_array().into_iter().flatten().filter_map(|part| part["text"].as_str()).collect::<Vec<_>>().join("\n");
        if !text.trim().is_empty() {
            recent.push_back((role.to_string(), text));
            if recent.len() > 2 { recent.pop_front(); }
        }
    }
    if matched_turn { recent.into() } else { Vec::new() }
}

fn claude_question(v: &Value, session: &str) -> Option<String> {
    if session.is_empty() || session.contains('/') || session.contains('\\') { return None; }
    let base = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".claude")))?.join("projects").canonicalize().ok()?;
    let path = PathBuf::from(field(v, &["transcript_path"])).canonicalize().ok()?;
    if !path.starts_with(base) || path.file_name()?.to_str()? != format!("{session}.jsonl") { return None; }
    let mut file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let offset = length.saturating_sub(2 * 1024 * 1024);
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut reader = BufReader::new(file);
    if offset > 0 { let _ = reader.read_line(&mut String::new()); }
    let answer = field(v, &["last_assistant_message"]);
    if answer.is_empty() { return None; }
    let mut current = String::new();
    let mut matched = None;
    for line in reader.lines().map_while(Result::ok) {
        let Ok(entry) = serde_json::from_str::<Value>(&line) else { continue; };
        if entry["sessionId"].as_str() != Some(session) { continue; }
        let message = &entry["message"];
        if entry["type"] == "user" && message["role"] == "user" {
            let text = content(&message["content"]);
            if !text.trim().is_empty() { current = text; }
        } else if entry["type"] == "assistant" && message["role"] == "assistant"
            && content(&message["content"]).trim() == answer.trim() && !current.is_empty() {
            if matched.is_some() { return None; }
            matched = Some(current.clone());
        }
    }
    matched
}

#[cfg(unix)]
fn alma_context(session: &str, event: &str) -> Option<(String, String, Vec<(String, String)>)> {
    use rusqlite::{Connection, OpenFlags};
    if session.is_empty() || event.is_empty() { return None; }
    #[cfg(target_os = "macos")]
    let path = home()?.join("Library/Application Support/alma/chat_threads.db");
    #[cfg(not(target_os = "macos"))]
    let path = home()?.join(".config/alma/chat_threads.db");
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).ok()?;
    db.busy_timeout(std::time::Duration::from_millis(50)).ok()?;
    let title = db.query_row("SELECT title FROM chat_threads WHERE id=?1", [session], |row| row.get::<_, String>(0)).unwrap_or_default();
    // Follow the reply's own parent chain: never pick a prompt from another branch or thread.
    let mut stmt = db.prepare("WITH RECURSIVE chain(id,parent_id,message,depth) AS (
        SELECT id,parent_id,message,0 FROM chat_messages WHERE thread_id=?1 AND id=?2
        UNION ALL SELECT p.id,p.parent_id,p.message,chain.depth+1 FROM chat_messages p
        JOIN chain ON p.id=chain.parent_id WHERE p.thread_id=?1 AND chain.depth<100)
        SELECT message FROM chain WHERE depth>0 AND json_extract(message,'$.role') IN ('user','assistant') ORDER BY depth LIMIT 3").ok()?;
    let messages = stmt.query_map([session,event], |row| row.get::<_, String>(0)).ok()?;
    let mut question = String::new();
    let mut context = Vec::new();
    for message in messages.flatten() {
        let Ok(message) = serde_json::from_str::<Value>(&message) else { continue; };
        let role = message["role"].as_str().unwrap_or("");
        let text = content(&message["parts"]);
        if text.trim().is_empty() { continue; }
        if question.is_empty() && role == "user" { question = text; }
        else if !question.is_empty() { context.push((role.to_owned(), text)); }
    }
    context.reverse();
    Some((title, question, context))
}

pub(super) fn format(agent: &str, v: &Value, language: &str) -> Option<(String, String, String)> {
    let typ = field(v, &["type", "hook_event_name"]);
    if !matches!((agent, typ),
        ("codex", "agent-turn-complete") | ("claude", "Stop") | ("gemini", "AfterAgent") |
        ("opencode", "session.idle") | ("pi", "agent_end") | ("omp", "agent_end") |
        ("kimi", "Stop") | ("kimi", "StopFailure") | ("zcode", "Stop") |
        ("alma", "chat.message.didReceive") | ("hermes", "on_session_end") |
        ("openclaw", "agent_end") | ("dsh", "turn/end")) { return None; }
    let session = field(v, &["thread-id", "session_id", "sessionID"]).to_owned();
    let cwd = field(v, &["cwd"]);
    let project = Path::new(cwd).file_name().and_then(|name| name.to_str()).unwrap_or("");
    let mut task = question(v);
    let mut history = if agent == "codex" { codex_context(&session, field(v, &["turn-id"])) } else { Vec::new() };
    if agent == "claude" && task.is_empty() { task = claude_question(v, &session).unwrap_or_default(); }
    let mut title = field(v, &["session_title", "thread_title", "title"]).to_owned();
    if agent == "codex" && title.is_empty() { title = codex_title(&session).unwrap_or_default(); }
    #[cfg(unix)]
    if agent == "alma" {
        if let Some((name, prompt, context)) = alma_context(&session, field(v, &["event_id"])) {
            if title.is_empty() { title = name; }
            if task.is_empty() { task = prompt; }
            history = context;
        }
    }
    let zh = language == "zh-CN";
    let failed = (agent == "kimi" && typ == "StopFailure") || v["failed"].as_bool() == Some(true) || v["completed"].as_bool() == Some(false);
    let interrupted = v["interrupted"].as_bool() == Some(true);
    let state = if zh { if interrupted { "已中断" } else if failed { "失败" } else { "已完成" } }
        else if interrupted { "Interrupted" } else if failed { "Failed" } else { "Completed" };
    let session_title = single_line(&title, 100);
    let title = if session_title.is_empty() { single_line(&task, 100) } else { session_title.clone() };
    let report_label = if zh { "EMA 报告" } else { "EMA Report" };
    let subject = if title.is_empty() { format!("{report_label} | {} | {state}", agent.to_ascii_uppercase()) }
        else { format!("{report_label} | {} | {title}", agent.to_ascii_uppercase()) };
    let mut answer = excerpt(field(v, &["last-assistant-message", "last_assistant_message", "prompt_response", "response"]), 10000, zh);
    let mut task = excerpt(&task, 4000, zh);
    let truncated = if zh { "\n…（内容已截断）" } else { "\n… (truncated)" };
    if v["response_truncated"] == true && !answer.ends_with(truncated) { answer.push_str(truncated); }
    if v["prompt_truncated"] == true && !task.ends_with(truncated) { task.push_str(truncated); }
    if history.is_empty() {
        history = v["context_messages"].as_array().into_iter().flatten().filter_map(|message| {
            let role = message["role"].as_str()?;
            let text = message["text"].as_str()?;
            if matches!(role, "user" | "assistant") && !text.trim().is_empty() { Some((role.to_owned(), text.to_owned())) } else { None }
        }).collect();
    }
    let context = if history.is_empty() { excerpt(field(v, &["context"]), 2000, zh) } else {
        let messages = history.iter().rev().take(2).map(|(role, text)| {
            let label = if zh { if role == "user" { "用户" } else { "Agent" } }
                else if role == "user" { "User" } else { "Agent" };
            format!("{label}: {}", excerpt(text, 1000, zh))
        }).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n\n");
        excerpt(&messages, 2200, zh)
    };
    let session_label: String = session.chars().filter(|c| !c.is_control()).take(2048).collect();
    let project = single_line(project, 100);
    let empty = if zh { "未提供" } else { "Unavailable" };
    let no_answer = if zh { "未提供最终回复。" } else { "Final response unavailable." };
    // Keep an empty machine-readable Session value when the native ID is absent.
    let session_line = if session_label.is_empty() {
        if zh { "\n会话：未提供" } else { "\nSession unavailable." }
    } else { "" };
    let mut body = if zh {
        format!("Agent: {agent}\n状态：{state}\n项目：{}\nSession: {}{}\n\n问题：\n{}", if project.is_empty(){empty}else{&project}, session_label, session_line, if task.is_empty(){empty}else{&task})
    } else {
        format!("Agent: {agent}\nStatus: {state}\nProject: {}\nSession: {}{}\n\nQuestion:\n{}", if project.is_empty(){empty}else{&project}, session_label, session_line, if task.is_empty(){empty}else{&task})
    };
    if !session_title.is_empty() { body.push_str(&format!("\n\n{} {}", if zh {"会话标题："} else {"Session title:"}, session_title)); }
    body.push_str(&format!("\n\n{}\n{}", if zh {"上下文："} else {"Context:"}, if context.is_empty(){empty}else{&context}));
    body.push_str(&format!("\n\n{}\n{}", if zh {"执行结果："} else {"Result:"}, if answer.is_empty(){no_answer}else{&answer}));
    Some((session, subject, body))
}
