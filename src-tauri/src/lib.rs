use std::sync::{Arc, Mutex};

#[derive(Default)]
struct UpdateGate(Mutex<Option<tokio::sync::OwnedRwLockWriteGuard<()>>>);

#[tauri::command]
fn prepare_app_update(state: tauri::State<'_, Arc<Gateway>>, gate: tauri::State<'_, UpdateGate>) -> Result<(), String> {
    let mut held = gate.0.lock().map_err(|e| e.to_string())?;
    if held.is_some() { return Err("Update installation already in progress / 更新安装已在进行".into()); }
    *held = Some(state.prepare_update().map_err(|e| e.to_string())?);
    Ok(())
}

#[tauri::command]
fn release_app_update(gate: tauri::State<'_, UpdateGate>) -> Result<(), String> {
    gate.0.lock().map_err(|e| e.to_string())?.take();
    Ok(())
}
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
fn has_mail_secret() -> bool {
    secret("mail").is_ok()
}
#[tauri::command]
fn get_secret(name: String) -> Result<String, String> {
    secret(&name).map_err(|e| e.to_string())
}
#[tauri::command]
fn get_stats(state: tauri::State<'_, Arc<Gateway>>) -> Result<ema_core::db::Stats,String> {
    state.stats().map_err(|e| e.to_string())
}
#[tauri::command]
fn get_mail_history(state: tauri::State<'_, Arc<Gateway>>, search: String, offset: i64) -> Result<Vec<ema_core::db::MailRecord>, String> {
    state.database.mail_history(&search, offset).map_err(|e| e.to_string())
}
#[tauri::command]
async fn receive_mail_now(state: tauri::State<'_, Arc<Gateway>>) -> Result<(), String> {
    state.inner().clone().receive_mail_now().await.map_err(|e| format!("{e:#}"))
}
#[tauri::command]
fn get_mail_reply_context(state: tauri::State<'_, Arc<Gateway>>, record_id: String) -> Result<Option<ema_core::ReplyContext>, String> {
    state.mail_reply_context(&record_id).map_err(|e|e.to_string())
}
#[tauri::command]
async fn reply_to_mail(state: tauri::State<'_, Arc<Gateway>>, record_id: String, request_id: String, body: String) -> Result<String, String> {
    state.inner().clone().reply_to_mail(&record_id,&request_id,&body).await.map_err(|e|format!("{e:#}"))
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
async fn check_mail(state: tauri::State<'_, Arc<Gateway>>, settings: Settings, password: Option<String>) -> Result<(), String> {
    let _activity = state.begin_activity().map_err(|e| e.to_string())?;
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
/// SMTP test uses saved mailbox settings, not caller-provided destination policy.
#[tauri::command]
async fn send_test_mail(state: tauri::State<'_, Arc<Gateway>>, recipient: String) -> Result<(), String> {
    let _activity = state.begin_activity().map_err(|e| e.to_string())?;
    let cfg = state.settings();
    let address = recipient.trim().to_ascii_lowercase();
    if address.is_empty() || address.contains('\r') || address.contains('\n')
        || !cfg.is_allowed(&address) {
        return Err("Test recipient must be in the saved sender allowlist".into());
    }
    if cfg.mail.address.trim().is_empty() { return Err("Configure mailbox first".into()); }
    let mut check = cfg.clone();
    check.enabled = false;
    check.validate().map_err(|e| e.to_string())?;
    let password = secret("mail").map_err(|e| e.to_string())?;
    let outgoing = state.database.queue_connection_test(&cfg.mail.address, &address, &cfg.language).map_err(|e|e.to_string())?;
    let id = outgoing.id.clone();
    let mail_config = cfg.mail.clone();
    let pass = password.clone();
    let result = tauri::async_runtime::spawn_blocking(move || mail::send(&mail_config, &pass, &outgoing)).await;
    let error = match result {
        Ok(Ok(())) => None,
        Ok(Err(err)) => Some(mail::redact_smtp_error(&format!("{err:#}"),&password)),
        Err(err) => Some(mail::redact_smtp_error(&err.to_string(),&password)),
    };
    state.database.outbox_result(&id,error.as_deref(),0,false,&cfg.mail.address).map_err(|e|e.to_string())?;
    match error { Some(err) => Err(err), None => Ok(()) }
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
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(UpdateGate::default())
        .setup(|app| {
            let dir=app.path().app_data_dir()?;
            let gateway=Gateway::open(dir)?;
            app.manage(gateway.clone());
            tauri::async_runtime::spawn(gateway.run());
            let open=tauri::menu::MenuItem::with_id(app,"open","Open Settings",true,None::<&str>)?;
            let exit=tauri::menu::MenuItem::with_id(app,"quit","Quit EMA",true,None::<&str>)?;
            let menu=tauri::menu::Menu::with_items(app,&[&open,&exit])?;
            let tray=tauri::tray::TrayIconBuilder::new()
                .icon(tauri::include_image!("icons/tray/32x32.png"))
                .icon_as_template(cfg!(target_os="macos"))
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
            prepare_app_update,release_app_update,
            get_settings,save_settings,set_secret,has_mail_secret,get_secret,get_stats,agent_status,detect_agents,check_mail,send_test_mail,
            get_mail_history,receive_mail_now,get_mail_reply_context,reply_to_mail
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
