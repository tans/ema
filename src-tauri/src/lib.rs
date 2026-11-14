use std::sync::Arc;
use tauri::{Manager, WindowEvent};
use ema_core::{config::{Settings,save_secret,secret}, mail, Gateway};
use serde::Serialize;

#[derive(Serialize)]
struct AgentStatus {
    installed: bool,
    command: String,
}
#[tauri::command]
fn get_settings(state: tauri::State<'_, Arc<Gateway>>) -> Settings {
    state.settings()
}
#[tauri::command]
fn save_settings(state: tauri::State<'_, Arc<Gateway>>, settings: Settings) -> Result<(), String> {
    state.save(settings).map_err(|e| e.to_string())
}
#[tauri::command]
fn set_secret(name: String, value: String) -> Result<(), String> {
    save_secret(&name,&value).map_err(|e| e.to_string())
}
#[tauri::command]
fn get_stats(state: tauri::State<'_, Arc<Gateway>>) -> Result<ema_core::db::Stats,String> {
    state.stats().map_err(|e| e.to_string())
}
#[derive(Serialize)]
struct DetectedAgent { name: &'static str, installed: bool, supported: bool }

#[tauri::command]
fn detect_agents(state: tauri::State<'_, Arc<Gateway>>) -> Vec<DetectedAgent> {
    let command = state.settings().codex_command;
    let mut items = Vec::new();
    for (name, executable, supported) in [
        ("codex", command.as_str(), true), ("cc", "claude", false),
        ("opencode", "opencode", false), ("pi", "pi", false),
        ("omp", "omp", false), ("zcode", "zcode", false),
        ("kimi", "kimi", false), ("dsh", "dsh", false),
        ("hermes", "hermes", false), ("openclaw", "openclaw", false),
        ("gemini", "gemini", false),
    ] {
        items.push(DetectedAgent {
            name,
            installed: std::process::Command::new(executable).arg("--version")
                .output().map(|o| o.status.success()).unwrap_or(false),
            supported,
        });
    }
    items
}

#[tauri::command]
async fn check_mail(settings: Settings, password: Option<String>) -> Result<(), String> {
    // Testing a mailbox must not require enabling the Gateway or an allowlist.
    let mut check = settings;
    check.enabled = false;
    check.validate().map_err(|e| e.to_string())?;
    if check.mail.address.is_empty() || check.mail.username.is_empty()
        || check.mail.imap_host.is_empty() {
        return Err("Email address, login and IMAP host are required".into());
    }
    let pass = match password {
        Some(s) if !s.is_empty() => s,
        _ => secret("mail").map_err(|e| e.to_string())?,
    };
    let mail_config = check.mail.clone();
    tauri::async_runtime::spawn_blocking(move || mail::check_connection(&mail_config, &pass))
        .await.map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
#[tauri::command]
fn agent_status(state: tauri::State<'_, Arc<Gateway>>) -> AgentStatus {
    let command=state.settings().codex_command;
    let installed=std::process::Command::new(&command).arg("--version")
        .output().map(|o| o.status.success()).unwrap_or(false);
    AgentStatus{installed,command}
}

pub fn run() {
    tracing_subscriber::fmt::init();
    tauri::Builder::default()
        .setup(|app| {
            let dir=app.path().app_data_dir()?;
            let gateway=Gateway::open(dir)?;
            app.manage(gateway.clone());
            tauri::async_runtime::spawn(gateway.run());
            let open=tauri::menu::MenuItem::with_id(app,"open","Open Settings",true,None::<&str>)?;
            let exit=tauri::menu::MenuItem::with_id(app,"quit","Quit EMA",true,None::<&str>)?;
            let menu=tauri::menu::Menu::with_items(app,&[&open,&exit])?;
            let tray=tauri::tray::TrayIconBuilder::new()
                .tooltip("Email Agent Gateway").menu(&menu)
                .on_menu_event(|app,event| {
                    match event.id().as_ref() {
                        "open" => {
                            if let Some(w)=app.get_webview_window("main") {
                                let _=w.show();
                                let _=w.set_focus();
                            }
                        }
                        "quit" => app.exit(0),
                        _ => {}
                    }
                }).build(app)?;
            // Retain tray for the lifetime of the Tauri app.
            app.manage(tray);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,save_settings,set_secret,get_stats,agent_status,detect_agents,check_mail
        ])
        .on_window_event(|window,event| {
            if let WindowEvent::CloseRequested{api,..}=event {
                api.prevent_close();
                let _=window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to start EMA");
}
