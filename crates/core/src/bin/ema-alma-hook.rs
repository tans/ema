//! Manage Alma registration without starting EMA's mail workers.
#[cfg(unix)]
fn main() -> anyhow::Result<()> {
    use std::path::PathBuf;
    use ema_core::hooks_alma;
    let mut args = std::env::args().skip(1);
    let action = args.next().unwrap_or_else(|| "status".into());
    let home = PathBuf::from(std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is unset"))?);
    let root = args.next().map(PathBuf::from).unwrap_or_else(|| {
        #[cfg(target_os = "macos")]
        { home.join("Library/Application Support/xin.ema.gateway") }
        #[cfg(not(target_os = "macos"))]
        { home.join(".local/share/xin.ema.gateway") }
    });
    anyhow::ensure!(root.is_absolute(), "hook root must be absolute");
    anyhow::ensure!(args.next().is_none(), "usage: ema-alma-hook [install|uninstall|status] [absolute EMA data directory]");
    match action.as_str() {
        "install" => hooks_alma::install(&root, &home)?,
        "uninstall" => hooks_alma::uninstall(&root, &home)?,
        "status" => {},
        _ => anyhow::bail!("usage: ema-alma-hook [install|uninstall|status] [absolute EMA data directory]"),
    }
    println!("{}", serde_json::to_string_pretty(&hooks_alma::status(&root, &home)?)?);
    Ok(())
}
#[cfg(not(unix))]
fn main() {
    eprintln!("Alma command hooks currently require Unix");
    std::process::exit(1);
}
