use crate::config::Settings;

pub struct Decision { pub agent: String, pub project: String, pub task: String }

/// Only a leading plain-text Agent prefix followed by whitespace can create a task.
pub fn route(config: &Settings, subject: &str, body: &str) -> Option<Decision> {
    let (prefix, title) = subject.split_once(char::is_whitespace)?;
    if title.trim().is_empty() { return None; }
    let matched = config.agent_routes.iter().find(|r| r.prefix.eq_ignore_ascii_case(prefix)  )?;
    let task = format!("Email task: {}\n\n{}\n\nUse native memory for project context. Ask instead of guessing.", title.trim(), body);
    Some(Decision { agent: matched.agent.clone(), project: "_agent_memory".into(), task })
}
