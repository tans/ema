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
#[serde(default)]
pub struct Settings {
    pub language: String,
    pub mail: MailSettings,
    pub router: RouterSettings,
    pub allowlist: Vec<String>,
    pub projects: Vec<Project>,
    pub codex_command: String,
    pub workspace_write: bool,
    pub poll_seconds: u64,
    pub progress_minutes: Vec<u64>,
    pub enabled: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            language: "zh-CN".into(), mail: MailSettings::default(), router: RouterSettings::default(),
            allowlist: Vec::new(), projects: Vec::new(), codex_command: "codex".into(),
            workspace_write: false, poll_seconds: 30, progress_minutes: vec![10, 30, 60],
            enabled: false,
        }
    }
}
impl Settings {
    pub fn read(dir: &Path) -> Result<Self> {
        let path = dir.join("config.json");
        if !path.exists() { return Ok(Self::default()); }
        let settings: Self = serde_json::from_slice(&fs::read(path)?)?;
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
        let mut aliases = std::collections::HashSet::new();
        for p in &self.projects {
            if p.alias.is_empty() || !p.alias.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                bail!("project alias must be alphanumeric, hyphen or underscore");
            }
            if !aliases.insert(&p.alias) { bail!("duplicate project alias"); }
            if !p.path.canonicalize().context("project directory is not accessible")?.is_dir() {
                bail!("project must be a directory");
            }
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
