//! Alma's main-reply command hook. No server or mail transport is started here.
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::{fs, path::{Path, PathBuf}};
use crate::hooks::{atomic_write, install_shim};
use agent_hooks::util::{backup_once, shell_quote, surgical_write};

const EVENT: &str = "chat.message.didReceive";
fn config_path(home: &Path) -> PathBuf { home.join(".config/alma/hooks.json") }
fn disabled(root: &Path) -> PathBuf { root.join("hooks/alma.disabled") }
fn command(shim: &Path) -> String { format!("{} alma", shell_quote(&shim.to_string_lossy())) }
fn read(file: &Path) -> Result<Value> {
    if !file.exists() { return Ok(json!({})); }
    serde_json::from_slice(&fs::read(file)?).context("invalid Alma hooks JSON; preserving file")
}
fn save(file: &Path, doc: &Value) -> Result<()> {
    let old = if file.exists() { fs::read(file)? } else { Vec::new() };
    let bytes = serde_json::to_vec_pretty(doc)?;
    if bytes != old { backup_once(file, &old)?; atomic_write(file, &bytes)?; }
    Ok(())
}
fn groups(doc: &mut Value) -> Result<&mut Vec<Value>> {
    let obj = doc.as_object_mut().context("Alma hooks config must be an object")?;
    let hooks = obj.entry("hooks").or_insert_with(|| json!({})).as_object_mut()
        .context("Alma hooks must be an object")?;
    hooks.entry(EVENT).or_insert_with(|| json!([])).as_array_mut()
        .context("Alma main-reply hooks must be an array")
}
fn owned(entry: &Value, target: &str) -> bool {
    entry["type"] == "command" && entry["command"].as_str() == Some(target)
}

fn register_config(home: &Path, shim: &Path) -> Result<()> {
    let file = config_path(home);
    let target = command(shim);
    // Merge inside the surgical write so a concurrent edit to hooks.json is re-read and
    // re-merged rather than clobbered.
    surgical_write(&file, &|current| {
        let mut doc = if current.is_empty() { json!({}) } else {
            serde_json::from_slice(current)
                .context("invalid Alma hooks JSON; preserving file")?
        };
        let items = groups(&mut doc)?;
        if items.iter().any(|g| g["hooks"].as_array()
            .map(|a| a.iter().any(|h| owned(h, target.as_str()))).unwrap_or(false)) {
            return Ok(None);
        }
        items.push(json!({"matcher":".*","hooks":[{
            "type":"command","command":target.as_str(),"timeout":3000,"enabled":true
        }]}));
        let bytes = serde_json::to_vec_pretty(&doc)?;
        if bytes == current { return Ok(None); }
        Ok(Some(bytes))
    })
}
/// Explicit install may create a profile; it does not re-enable an existing disabled entry.
pub fn install(root: &Path, home: &Path) -> Result<()> {
    anyhow::ensure!(root.is_absolute(), "hook root must be absolute");
    let shim = install_shim(root)?;
    register_config(home, &shim)?;
    if disabled(root).exists() { fs::remove_file(disabled(root))?; }
    Ok(())
}
/// Remove this root's command only; shared shims and other agents remain installed.
pub fn uninstall(root: &Path, home: &Path) -> Result<()> {
    let file = config_path(home);
    let mut doc = read(&file)?;
    let target = command(&root.join("hooks/emit.sh"));
    if let Some(items) = doc.pointer_mut("/hooks/chat.message.didReceive").and_then(Value::as_array_mut) {
        items.retain_mut(|g| {
            let Some(hooks) = g.get_mut("hooks").and_then(Value::as_array_mut) else { return true; };
            let before = hooks.len();
            hooks.retain(|h| !owned(h, &target));
            before == hooks.len() || !hooks.is_empty()
        });
    }
    // Prevent the running gateway from reinstalling on its next reconciliation.
    atomic_write(&disabled(root), b"EMA-MANAGED-HOOK: Alma explicitly uninstalled\n")?;
    if file.exists() { save(&file, &doc)?; }
    Ok(())
}
pub fn status(root: &Path, home: &Path) -> Result<Value> {
    let file = config_path(home);
    let doc = read(&file)?;
    let shim = root.join("hooks/emit.sh");
    let target = command(&shim);
    let entries: Vec<_> = doc.pointer("/hooks/chat.message.didReceive").and_then(Value::as_array)
        .into_iter().flatten().flat_map(|g| g["hooks"].as_array().into_iter().flatten()
            .filter(|h| owned(h, &target)).map(move |h| (g, h))).collect();
    let enabled = entries.iter().any(|(g,h)| h["enabled"].as_bool() != Some(false)
        && g["matcher"].as_str().unwrap_or(".*") == ".*");
    let executable = {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(&shim).map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    };
    let active = fs::read_to_string(root.join("hooks/active")).ok()
        .and_then(|s| s.parse::<u64>().ok()).map(|n| {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
            now >= n && now - n <= 20
        }).unwrap_or(false);
    Ok(json!({"configPath":file,"command":target,"installed":!entries.is_empty(),
        "enabled":enabled,"executable":executable,"collectorActive":active,
        "reconciliationDisabled":disabled(root).exists()}))
}
