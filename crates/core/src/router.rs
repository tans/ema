use anyhow::{bail, Result};
use crate::config::Settings;

pub struct Decision { pub agent: String, pub project: String, pub task: String }

/// New sessions rely on the installed CLI's own instructions, prior sessions and
/// available native memory. This function deliberately makes no external LLM API call.
pub async fn route(config: &Settings, subject: &str, body: &str) -> Result<Decision> {
    if subject.starts_with('[') {
        if let Some(end) = subject.find(']') {
            if let Some((agent, alias)) = subject[1..end].split_once(':') {
                if agent == "codex" && config.projects.iter().any(|p| p.alias == alias) {
                    return Ok(Decision {
                        agent: "codex".into(), project: alias.into(), task: body.into(),
                    });
                }
                bail!("explicit project route is not authorized");
            }
        }
    }

    // Never upgrade permissions merely because a remembered path is mentioned.
    // A legacy write-enabled setup can only run inside an explicitly trusted project.
    if config.workspace_write {
        if config.projects.len() == 1 {
            return Ok(Decision { agent: "codex".into(),
                project: config.projects[0].alias.clone(), task: body.into() });
        }
        bail!("write mode requires an explicit [codex:approved-project] route");
    }

    let instructions = "New email session. Reuse your own available native instructions,         memory and past context to identify the relevant project. If the project or         intended operation is unclear, ask the sender to clarify rather than guessing.         Prior context is not permission to write files. Keep the answer concise and         suitable for an email reply.";
    let task = format!("{instructions}\n\nEmail subject: {subject}\n\nRequest:\n{body}");
    Ok(Decision { agent: "codex".into(), project: "_agent_memory".into(), task })
}
