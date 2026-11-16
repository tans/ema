//! Shared presentation and self-identification for every EMA outbound email.
use crate::db::Session;

pub const RESULT_LIMIT: usize = 12_000;
pub const TASK_LIMIT: usize = 2_000;
pub const KINDS: &[&str] = &[
    "new_task_ack",
    "resume_ack",
    "progress",
    "result",
    "error",
    "hook",
    "test",
];

pub fn text<'a>(language: &str, zh: &'a str, en: &'a str) -> &'a str {
    if language == "zh-CN" {
        zh
    } else {
        en
    }
}
pub fn excerpt(value: &str, limit: usize, language: &str) -> String {
    let clean: String = value
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect();
    let mut chars = clean.trim().chars();
    let mut result: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        result.push_str(text(language, "\n…（内容已截断）", "\n… (truncated)"));
    }
    result
}
pub fn single_line(value: &str, limit: usize) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect()
}
pub fn reply_subject(subject: &str) -> String {
    let mut subject = subject.trim();
    while subject
        .get(..3)
        .is_some_and(|s| s.eq_ignore_ascii_case("re:"))
    {
        subject = subject[3..].trim_start();
    }
    format!("Re: {}", single_line(subject, 100))
}
pub fn marker(kind: &str, mid: &str) -> String {
    format!("EMA-Message: v=1; kind={kind}; id={mid}")
}
/// Only a standalone first line is a marker. Quoted/embedded markers are user content.
pub fn marker_id(body: &str) -> Option<&str> {
    let line = body.lines().next()?;
    let rest = line.strip_prefix("EMA-Message: v=1; kind=")?;
    let (kind, mid) = rest.split_once("; id=")?;
    (KINDS.contains(&kind)
        && mid.starts_with("<ema-")
        && mid.ends_with('>')
        && !mid.chars().any(char::is_whitespace))
    .then_some(mid)
}
pub fn envelope(kind: &str, mid: &str, body: &str) -> String {
    let body = crate::mail::redact_access_codes(body);
    let line = marker(kind, mid);
    if body.lines().next() == Some(line.as_str()) {
        body.to_owned()
    } else {
        format!("{line}\n\n{body}")
    }
}
pub fn render(
    language: &str,
    summary: &str,
    fields: &[(&str, &str)],
    sections: &[(&str, &str)],
    next: &str,
    trace: &[(&str, &str)],
) -> String {
    let separator = text(language, "：", ": ");
    let mut body = summary.to_string();
    for (label, value) in fields.iter().filter(|(_, v)| !v.trim().is_empty()) {
        body.push_str(&format!("\n{label}{separator}{}", single_line(value, 2048)));
    }
    for (label, value) in sections.iter().filter(|(_, v)| !v.trim().is_empty()) {
        body.push_str(&format!("\n\n{label}{separator}\n{value}"));
    }
    body.push_str(&format!(
        "\n\n{}{separator}\n{next}",
        text(language, "下一步", "Next step")
    ));
    if trace.iter().any(|(_, v)| !v.is_empty()) {
        body.push_str(&format!(
            "\n\n{}{separator}",
            text(language, "追踪信息", "Tracking")
        ));
        for (label, value) in trace.iter().filter(|(_, v)| !v.is_empty()) {
            body.push_str(&format!("\n{label}{separator}{}", single_line(value, 2048)));
        }
    }
    body
}
pub fn task_report(
    language: &str,
    session: &Session,
    job: &str,
    summary: &str,
    status: &str,
    sections: &[(&str, &str)],
    next: &str,
) -> String {
    render(
        language,
        summary,
        &[
            (text(language, "状态", "Status"), status),
            ("Agent", &session.agent),
            (
                text(language, "项目", "Project"),
                if session.project == "_agent_memory" {
                    ""
                } else {
                    &session.project
                },
            ),
            (
                text(language, "运行目录", "Working directory"),
                session.cwd.as_deref().unwrap_or(""),
            ),
        ],
        sections,
        next,
        &[
            (text(language, "任务 ID", "Task ID"), job),
            (text(language, "EMA 会话 ID", "EMA session ID"), &session.id),
            (
                text(language, "Agent 会话 ID", "Agent session ID"),
                session.agent_session.as_deref().unwrap_or(""),
            ),
        ],
    )
}
pub fn route_error(
    language: &str,
    agent: &str,
    session: &str,
    task: &str,
    code: &str,
    reason: &str,
    next: &str,
) -> String {
    render(
        language,
        text(
            language,
            "任务未受理，未启动 Agent。",
            "Task not accepted. No Agent was started.",
        ),
        &[
            ("Agent", agent),
            (text(language, "错误码", "Error code"), code),
        ],
        &[
            (text(language, "原因", "Reason"), reason),
            (
                text(language, "本次指令", "Instruction"),
                &excerpt(task, TASK_LIMIT, language),
            ),
        ],
        next,
        &[(text(language, "Agent 会话 ID", "Agent session ID"), session)],
    )
}
