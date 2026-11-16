//! The local spool shim every agent hook funnels into.
//!
//! The shim is the only code that runs inside agent processes' hook callbacks.
//! It never touches the network or reads credentials: it writes at most 64KiB
//! of the payload to a private spool directory, and only while the collector
//! is active (20-second window around the last `tick`).
use anyhow::Result;
use std::path::{Path, PathBuf};
use crate::util::{owned_executable, shell_quote};

/// Build the shim source for the given (ordered) agent dispatch list.
pub fn template(active: &Path, spool: &Path, agents: &[&str]) -> String {
    const TEMPLATE: &str = r###"#!/bin/sh
# EMA-MANAGED-HOOK: local spool; no SMTP credentials, no remote network.
ACTIVE=__ACTIVE__
SPOOL=__SPOOL__
case "$1" in __AGENTS__) ;; *) exit 0 ;; esac
[ -r "$ACTIVE" ] || exit 0
LAST="$(cat "$ACTIVE" 2>/dev/null)"
NOW="$(date +%s)"
case "$LAST" in ''|*[!0-9]*) exit 0 ;; esac
[ "$((NOW-LAST))" -ge 0 ] && [ "$((NOW-LAST))" -le 20 ] || exit 0
TMP="$(mktemp "$SPOOL/$1.XXXXXXXX")" || exit 0
chmod 600 "$TMP" 2>/dev/null
if [ "$1" = codex ]; then printf '%s' "$2" | head -c 65536 > "$TMP"
else head -c 65536 > "$TMP"; fi
if [ -s "$TMP" ]; then mv "$TMP" "$TMP.ready" 2>/dev/null || rm -f "$TMP"
else rm -f "$TMP"; fi
exit 0
"###;
    TEMPLATE
        .replace("__AGENTS__", &agents.join("|"))
        .replace("__ACTIVE__", &shell_quote(&active.to_string_lossy()))
        .replace("__SPOOL__", &shell_quote(&spool.to_string_lossy()))
}

#[cfg(unix)]
fn private_dir(p: &Path) -> Result<()> {
    std::fs::create_dir_all(p)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

/// Install (or refresh) the managed shim under `root/hooks/emit.sh` and
/// ensure the spool directories exist. Refuses to take over a non-EMA file.
#[cfg(unix)]
pub fn install(root: &Path, agents: &[&str]) -> Result<PathBuf> {
    let dir = root.join("hooks");
    private_dir(&dir)?;
    private_dir(&dir.join("inbox"))?;
    let shim = dir.join("emit.sh");
    let text = template(&dir.join("active"), &dir.join("inbox"), agents);
    let _ = owned_executable(&shim, &text, 0o700)?;
    Ok(shim)
}
