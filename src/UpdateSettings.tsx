import { invoke, isTauri } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { useEffect, useState, useSyncExternalStore } from "react";
import "daisyui/components/progress.css";
import { notify } from "./Toast";
import { AppUpdater } from "./updater";

const updater = new AppUpdater({
  check: () => check({ timeout: 15_000 }),
  prepare: () => invoke("prepare_app_update"),
  release: () => invoke("release_app_update"),
  restart: relaunch,
  notify
});
export function useAppUpdater(language: "zh-CN" | "en", enabled: boolean) {
  updater.language = language;
  const state = useSyncExternalStore(updater.subscribe, updater.snapshot);
  useEffect(() => { if (enabled && isTauri()) void updater.start(); }, [enabled]);
  return state;
}
const labels = {
  "zh-CN": {
    title: "软件更新", current: "当前版本", idle: "启动时后台检查更新，也可以在这里手动检查。", check: "检查更新", checking: "正在检查…",
    available: "可用版本", download: "下载更新", downloading: "正在下载并验证签名…", ready: "已下载并验证签名，等待安装。",
    install: "安装并重启", installing: "正在安装或重启…", installed: "更新已安装，等待重启。", restart: "重试重启", retry: "重试",
    confirm: "确认安装并重启？运行中的邮件或 Agent 任务会阻止安装；排队中的任务将在重启后继续。", confirmRestart: "确认重启 EMA？运行中的邮件或 Agent 任务会阻止重启。", cancel: "稍后", proceed: "确认", dirty: "请先保存或放弃其他设置的更改，再安装或重启。",
    policy: "补丁版本仅更新 Apple Silicon；Intel Mac、Windows 和 Linux 在次版本更新。Linux 自动更新仅支持 AppImage。",
    notes: "版本说明"
  },
  en: {
    title: "Updates", current: "Current version", idle: "Updates are checked in the background at startup. You can also check here.", check: "Check for updates", checking: "Checking…",
    available: "Available version", download: "Download update", downloading: "Downloading and verifying signature…", ready: "Downloaded and verified. Ready to install.",
    install: "Install and restart", installing: "Installing or restarting…", installed: "Update installed. Restart pending.", restart: "Retry restart", retry: "Retry",
    confirm: "Install and restart EMA? Active mail or Agent work will block installation. Queued tasks resume after restart.", confirmRestart: "Restart EMA? Active mail or Agent work will block restart.", cancel: "Later", proceed: "Confirm", dirty: "Save or discard other settings changes before installing or restarting.",
    policy: "Patch updates support Apple Silicon only. Intel Mac, Windows and Linux update on minor releases. Linux auto-update supports AppImage only.",
    notes: "Release notes"
  }
};
export default function UpdateSettings({ language, blocked }: { language: "zh-CN" | "en"; blocked: boolean }) {
  const state = useSyncExternalStore(updater.subscribe, updater.snapshot);
  const [current, setCurrent] = useState("—");
  const [confirming, setConfirming] = useState(false);
  useEffect(() => { if (isTauri()) void getVersion().then(setCurrent).catch(() => {}); }, []);
  const t = labels[language];
  const downloading = state.phase === "downloading";
  const ready = state.phase === "ready" || state.phase === "installed";
  const percent = state.total ? Math.min(100, Math.floor(state.downloaded / state.total * 100)) : undefined;
  return <section aria-label={t.title}>
    <p className="notice">{t.current}: {current}</p>
    <p className="field-hint">{state.version ? `${t.available}: ${state.version}` : t.idle}</p>
    {downloading && <div className="update-progress" role="status">
      <p className="field-hint">{t.downloading} {percent === undefined ? `${(state.downloaded / 1048576).toFixed(1)} MB` : `${percent}%`}</p>
      {percent !== undefined && <progress className="progress" value={percent} max={100} aria-label={t.downloading}/>}
    </div>}
    {ready && <p className="field-hint">{state.phase === "installed" ? t.installed : t.ready}</p>}
    {state.phase === "installing" && <p className="field-hint" role="status">{t.installing}</p>}
    <div className="settings-footer">
      {(state.phase === "idle" || state.phase === "available" || state.phase === "checking") && <button type="button" className="btn btn-ghost" disabled={state.phase === "checking"} onClick={() => { void updater.check(); }}>
        {state.phase === "checking" ? t.checking : t.check}
      </button>}
      {state.phase === "available" && <button type="button" className="btn primary-button" onClick={() => { void updater.download(); }}>{state.failed ? `${t.retry} · ${t.download}` : t.download}</button>}
      {ready && !confirming && <button type="button" className="btn primary-button" disabled={blocked} onClick={() => setConfirming(true)}>{state.phase === "installed" ? t.restart : state.failed ? `${t.retry} · ${t.install}` : t.install}</button>}
      {ready && confirming && <div>
        <p className="notice">{state.phase === "installed" ? t.confirmRestart : t.confirm}</p>
        <button type="button" className="btn primary-button" disabled={blocked} onClick={() => { setConfirming(false); void updater.install(); }}>{t.proceed}</button>
        <button type="button" className="btn btn-ghost" onClick={() => setConfirming(false)}>{t.cancel}</button>
      </div>}
      {ready && blocked && <p className="field-hint">{t.dirty}</p>}
    </div>
    {state.body && <details className="advanced"><summary>{t.notes}</summary><p className="notice" style={{ whiteSpace: "pre-wrap" }}>{state.body}</p></details>}
    <p className="field-hint">{t.policy}</p>
  </section>;
}
