//! Shared, conservative file-rewriting helpers.
//!
//! Every adapter must go through these: they enforce the marker-based
//! ownership rules, 0600 permissions, atomic renames, the one-time backup of
//! user-owned files before first modification, and conflict-tolerant
//! read-modify-write (agents often rewrite their own configs concurrently).
use anyhow::Result;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

/// Marker that identifies a file or config entry as EMA-managed.
pub const MARKER: &str = "EMA-MANAGED-HOOK";

fn stage(p: &Path, bytes: &[u8]) -> Result<std::path::PathBuf> {
    use std::io::Write;
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = p.with_extension(format!("ema-{}", uuid::Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(tmp)
}

/// Atomically write `bytes` to `path` (0600 on unix). Parent dirs created.
pub fn atomic_write(p: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = stage(p, bytes)?;
    fs::rename(tmp, p)?;
    Ok(())
}

/// Keep a one-time `.ema-before-hook.bak` of a file before first modification.
pub fn backup_once(path: &Path, old: &[u8]) -> Result<()> {
    if !old.is_empty() {
        let backup = path.with_extension("ema-before-hook.bak");
        if !backup.exists() {
            atomic_write(&backup, old)?;
        }
    }
    Ok(())
}

/// Idempotent read-modify-write that survives an agent rewriting the same file
/// underneath us (e.g. Codex touching its own config.toml while EMA reconciles).
/// `plan` is called with the current bytes and returns Ok(None) when the file already
/// has what we want, or Ok(Some(bytes)) with the desired new content. Before the
/// rename we re-read the live file; if it changed since our read, we re-plan on the
/// fresh content, up to a bounded number of replays. External edits win if we keep
/// losing the race; a parse failure in `plan` aborts without touching the file.
pub fn surgical_write(p: &Path, plan: &dyn Fn(&[u8]) -> Result<Option<Vec<u8>>>) -> Result<()> {
    const MAX_REPLAN: u32 = 3;
    for _ in 0..MAX_REPLAN {
        let current = if p.exists() {
            fs::read(p)?
        } else {
            Vec::new()
        };
        let next = match plan(&current)? {
            None => return Ok(()),
            Some(bytes) => bytes,
        };
        if next == current {
            return Ok(());
        }
        let tmp = stage(p, &next)?;
        let fresh = if p.exists() {
            fs::read(p).unwrap_or_default()
        } else {
            Vec::new()
        };
        if fresh == current {
            backup_once(p, &current)?;
            fs::rename(tmp, p)?;
            return Ok(());
        }
        // The file changed under us; drop the staged copy and replay on fresh content.
        let _ = fs::remove_file(tmp);
    }
    anyhow::bail!("write to {} kept conflicting with external edits", p.display())
}

pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Write a fully EMA-owned, non-executable file (plugins, extensions).
/// No-ops when identical; never overwrites a file that lacks the EMA marker.
/// Returns true when the file was (re)written.
pub fn owned_file(path: &Path, content: &str) -> Result<bool> {
    let wrote = std::cell::Cell::new(false);
    surgical_write(path, &|current| {
        if !current.is_empty() && !String::from_utf8_lossy(current).contains(MARKER) {
            tracing::warn!(file=%path.display(), "existing user plugin not overwritten");
            return Ok(None);
        }
        if current == content.as_bytes() {
            return Ok(None);
        }
        wrote.set(true);
        Ok(Some(content.as_bytes().to_vec()))
    })?;
    Ok(wrote.get())
}

/// Write a fully EMA-owned executable file (the spool shim). Same ownership
/// rules as [`owned_file`], plus explicit permission bits.
#[cfg(unix)]
pub fn owned_executable(path: &Path, content: &str, mode: u32) -> Result<bool> {
    let wrote = owned_file(path, content)?;
    if wrote {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    Ok(wrote)
}

/// True when `file` exists, is readable text and carries the EMA marker.
pub fn is_managed_file(path: &Path) -> bool {
    fs::read_to_string(path).map(|t| t.contains(MARKER)).unwrap_or(false)
}

/// Append (or confirm) one command hook entry under `settings.json`-style
/// layouts: { "hooks": { "<event>": [ { "hooks": [ {..} ] } ] } }.
/// Idempotent on the `ema-report` hook name; preserves everything else and
/// refuses to touch unreadable/invalid JSON. Returns true when written.
pub fn json_hook(file: &Path, event: &str, hook: &Value) -> Result<bool> {
    let wrote = std::cell::Cell::new(false);
    surgical_write(file, &|current| {
        let mut doc: Value = if current.is_empty() {
            json!({})
        } else {
            serde_json::from_slice(current)
                .map_err(|e| anyhow::anyhow!("invalid JSON in {}, preserving: {e}", file.display()))?
        };
        let Some(obj) = doc.as_object_mut() else {
            return Ok(None);
        };
        let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
        let Some(events) = hooks.as_object_mut() else {
            return Ok(None);
        };
        let list = events.entry(event).or_insert_with(|| json!([]));
        let Some(items) = list.as_array_mut() else {
            return Ok(None);
        };
        // Avoid a duplicate EMA hook, including a previous installation pointing to the same shim.
        if items.iter().any(|group| group["hooks"].as_array().map(|arr| arr.iter().any(|h|
            h["name"].as_str() == Some("ema-report"))).unwrap_or(false)) {
            return Ok(None);
        }
        items.push(hook.clone());
        wrote.set(true);
        Ok(Some(serde_json::to_vec_pretty(&doc)?))
    })?;
    Ok(wrote.get())
}
