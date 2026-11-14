use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, path::{Path, PathBuf}};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MailSettings {
    pub provider: String,
    pub address: String,
    pub username: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
}
impl Default for MailSettings {
    fn default() -> Self {
        Self { provider: "custom".into(), address: String::new(), username: String::new(), imap_host: String::new(),
            imap_port: 993, smtp_host: String::new(), smtp_port: 465 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RouterSettings {
    pub base_url: String,
    pub model: String,
}
impl Default for RouterSettings {
    fn default() -> Self {
        Self { base_url: "https://api.openai.com/v1".into(), model: String::new() }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub alias: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentRoute {
    pub agent: String,
    pub prefix: String,
    pub enabled: bool,
}
pub fn default_routes() -> Vec<AgentRoute> {
    vec![
    AgentRoute { agent: "codex".into(), prefix: "codex".into(), enabled: true },
    AgentRoute { agent: "cc".into(), prefix: "cc".into(), enabled: false },
    AgentRoute { agent: "opencode".into(), prefix: "opencode".into(), enabled: false },
    AgentRoute { agent: "pi".into(), prefix: "pi".into(), enabled: false },
    AgentRoute { agent: "omp".into(), prefix: "omp".into(), enabled: false },
    AgentRoute { agent: "zcode".into(), prefix: "zcode".into(), enabled: false },
    AgentRoute { agent: "kimi".into(), prefix: "kimi".into(), enabled: false },
    AgentRoute { agent: "dsh".into(), prefix: "dsh".into(), enabled: false },
    AgentRoute { agent: "hermes".into(), prefix: "hermes".into(), enabled: false },
    AgentRoute { agent: "openclaw".into(), prefix: "openclaw".into(), enabled: false },
    AgentRoute { agent: "gemini".into(), prefix: "gemini".into(), enabled: false }
    ]
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub language: String,
    pub mail: MailSettings,
    pub router: RouterSettings,
    pub allowlist: Vec<String>,
    pub agent_routes: Vec<AgentRoute>,
    pub projects: Vec<Project>,
    pub codex_command: String,
    pub workspace_write: bool,
    pub poll_seconds: u64,
    pub progress_minutes: Vec<u64>,
    pub enabled: bool,
    pub onboarding_completed: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            language: "zh-CN".into(), mail: MailSettings::default(), router: RouterSettings::default(),
            allowlist: Vec::new(), agent_routes: default_routes(), projects: Vec::new(), codex_command: "codex".into(),
            workspace_write: false, poll_seconds: 30, progress_minutes: vec![10, 30, 60],
            enabled: false, onboarding_completed: false,
        }
    }
}
impl Settings {
    pub fn read(dir: &Path) -> Result<Self> {
        let path = dir.join("config.json");
        if !path.exists() { return Ok(Self::default()); }
        let bytes = fs::read(path)?;
        let mut settings: Self = serde_json::from_slice(&bytes)?;
        // Existing configured users should not be forced through new onboarding.
        let json: serde_json::Value = serde_json::from_slice(&bytes)?;
        if json.get("onboarding_completed").is_none()
            && !settings.mail.address.trim().is_empty() && !settings.allowlist.is_empty() {
            settings.onboarding_completed = true;
        }
        Ok(settings)
    }
    pub fn validate(&self) -> Result<()> {
        if !["zh-CN", "en"].contains(&self.language.as_str()) { bail!("unsupported language"); }
        if !["qq", "custom"].contains(&self.mail.provider.as_str()) { bail!("unsupported mail provider"); }
        if self.mail.provider == "qq" && (self.mail.imap_host != "imap.qq.com"
            || self.mail.imap_port != 993 || self.mail.smtp_host != "smtp.qq.com"
            || self.mail.smtp_port != 465 || self.mail.username != self.mail.address) {
            bail!("QQ Mail requires the preset TLS hosts and full mailbox as username");
        }
        if self.poll_seconds < 15 || self.poll_seconds > 3600 {
            bail!("poll_seconds must be between 15 and 3600");
        }
        if self.codex_command.trim().is_empty() { bail!("Codex executable is required"); }
        if self.allowlist.iter().any(|v| !v.contains('@') || (v.contains('\r') || v.contains('\n'))) {
            bail!("allowlist must contain valid email addresses");
        }
        let mut prefixes = std::collections::HashSet::new();
        let mut agents = std::collections::HashSet::new();
        for route in &self.agent_routes {
            let prefix = route.prefix.trim().to_ascii_lowercase();
            if route.agent.is_empty() || !["codex","cc","opencode","pi","omp","zcode","kimi","dsh","hermes","openclaw","gemini"].contains(&route.agent.as_str()) {
                bail!("unsupported agent route");
            }
            if prefix.is_empty() || route.prefix != route.prefix.trim() || prefix.chars().any(|c| c.is_whitespace() || c.is_control()) {
                bail!("agent prefix must be a non-empty single token");
            }
            if !prefixes.insert(prefix) || !agents.insert(route.agent.as_str()) { bail!("duplicate agent or prefix"); }
            if route.enabled && route.agent != "codex" { bail!("execution adapter unavailable for {}", route.agent); }
        }
        let mut aliases = std::collections::HashSet::new();
        for p in &self.projects {
            if p.alias.is_empty() || !p.alias.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                bail!("project alias must be alphanumeric, hyphen or underscore");
            }
            if !aliases.insert(&p.alias) { bail!("duplicate project alias"); }
            // Retain legacy aliases even if their directories moved; resolve at execution time.
        }
        if self.enabled {
            if self.mail.address.trim().is_empty() || self.mail.imap_host.trim().is_empty()
                || self.mail.smtp_host.trim().is_empty() || self.mail.username.trim().is_empty() {
                bail!("configure mail account before enabling gateway");
            }
            if self.allowlist.is_empty() { bail!("at least one approved sender is required"); }
            if self.workspace_write && self.projects.is_empty() {
                bail!("workspace-write requires an explicitly authorized legacy project; memory never grants write access");
            }
        }
        Ok(())
    }
    pub fn write(&self, dir: &Path) -> Result<()> {
        self.validate()?;
        fs::create_dir_all(dir)?;
        let tmp = dir.join("config.json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        fs::rename(tmp, dir.join("config.json"))?;
        Ok(())
    }
    pub fn project(&self, alias: &str) -> Result<PathBuf> {
        if alias == "_agent_memory" {
            return std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
                .map(PathBuf::from).filter(|p| p.is_dir())
                .context("home directory unavailable for agent-native context");
        }
        let project = self.projects.iter().find(|p| p.alias == alias)
            .context("project is not authorized")?;
        let path = project.path.canonicalize()?;
        if !path.is_dir() { bail!("project directory unavailable"); }
        Ok(path)
    }
    pub fn is_allowed(&self, sender: &str) -> bool {
        self.allowlist.iter().any(|s| s.eq_ignore_ascii_case(sender))
    }
}

pub fn secret(name: &str) -> Result<String> {
    let env_name = match name {
        "mail" => "EMA_MAIL_PASSWORD", "llm" => "EMA_ROUTER_API_KEY",
        "access" => "EMA_MAIL_ACCESS_CODE", _ => bail!("unsupported secret"),
    };
    if let Ok(value) = std::env::var(env_name) {
        if !value.is_empty() { return Ok(value); }
    }
    Ok(keyring::Entry::new("tans.ema", name)?.get_password()?)
}

pub fn save_secret(name: &str, value: &str) -> Result<()> {
    if !["mail", "llm", "access"].contains(&name) { bail!("unsupported secret"); }
    if value.trim().is_empty() { bail!("secret cannot be empty"); }
    keyring::Entry::new("tans.ema", name)?.set_password(value)?;
    Ok(())
}
