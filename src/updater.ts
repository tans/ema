import type { Update } from "@tauri-apps/plugin-updater";

type Phase = "idle" | "checking" | "available" | "downloading" | "ready" | "installing" | "installed";
type Language = "zh-CN" | "en";
type UpdateHandle = Pick<Update, "version" | "body" | "download" | "install" | "close">;
type Dependencies = {
  check: () => Promise<UpdateHandle | null>;
  prepare: () => Promise<unknown>;
  release: () => Promise<unknown>;
  restart: () => Promise<unknown>;
  notify: (text: string, tone?: "info" | "error") => void;
};
type State = { phase: Phase; version?: string; body?: string; downloaded: number; total?: number; failed: boolean };

// One controller lives for the whole app, including when Settings is closed.
export class AppUpdater {
  language: Language = "zh-CN";
  private state: State = { phase: "idle", downloaded: 0, failed: false };
  private update: UpdateHandle | null = null;
  private started = false;
  private listeners = new Set<() => void>();
  constructor(private deps: Dependencies) {}
  snapshot = () => this.state;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };
  private set(next: Partial<State>) {
    this.state = { ...this.state, ...next };
    this.listeners.forEach(listener => listener());
  }
  private say(zh: string, en: string, error?: unknown) {
    this.deps.notify((this.language === "en" ? en : zh) + (error === undefined ? "" : `: ${String(error)}`), error === undefined ? "info" : "error");
  }
  async start() {
    if (this.started) return;
    this.started = true;
    await this.check(true);
  }
  async check(silent = false) {
    if (!["idle", "available"].includes(this.state.phase)) return;
    const previous = this.state.phase;
    this.set({ phase: "checking", failed: false });
    try {
      const update = await this.deps.check();
      await this.update?.close().catch(() => {});
      this.update = update;
      this.set({ phase: update ? "available" : "idle", version: update?.version, body: update?.body });
      if (update) this.say(`发现 EMA ${update.version}，请前往设置 → 软件更新。`, `EMA ${update.version} is available. Open Settings → Updates.`);
      else if (!silent) this.say("当前平台暂无新版本。", "No update is available for this platform.");
    } catch (error) {
      this.set({ phase: previous, failed: !silent });
      if (!silent) this.say("检查更新失败，请重试", "Update check failed. Please retry", error);
    }
  }
  async download() {
    if (this.state.phase !== "available" || !this.update) return;
    this.set({ phase: "downloading", downloaded: 0, total: undefined, failed: false });
    this.say("开始下载更新。", "Downloading update.");
    try {
      await this.update.download(event => {
        if (event.event === "Started") this.set({ total: event.data.contentLength });
        if (event.event === "Progress") this.set({ downloaded: this.state.downloaded + event.data.chunkLength });
        // Finished precedes signature verification; only the resolved promise is ready.
      }, { timeout: 120_000 });
      this.set({ phase: "ready" });
      this.say("更新已下载并通过签名验证，请确认安装并重启。", "Update downloaded and signature verified. Confirm installation and restart.");
    } catch (error) {
      this.set({ phase: "available", failed: true });
      this.say("下载或签名验证失败，请重试", "Download or signature verification failed. Please retry", error);
    }
  }
  async install() {
    if (!["ready", "installed"].includes(this.state.phase) || !this.update) return;
    const alreadyInstalled = this.state.phase === "installed";
    this.set({ phase: "installing", failed: false });
    let held = false;
    let installed = alreadyInstalled;
    try {
      await this.deps.prepare();
      held = true;
      this.say(alreadyInstalled ? "正在重启 EMA。" : "正在安装更新并重启 EMA。", alreadyInstalled ? "Restarting EMA." : "Installing update and restarting EMA.");
      if (!alreadyInstalled) {
        await this.update.install();
        installed = true;
      }
      await this.deps.restart();
      this.set({ phase: "installed" });
      // Keep the gate held until the process exits. Windows exits during install.
    } catch (error) {
      this.set({ phase: installed ? "installed" : "ready", failed: true });
      this.say(installed ? "更新已安装，重启失败，请重试重启" : "暂时无法安装更新，请重试", installed ? "Update installed, but restart failed. Retry restart" : "Cannot install update now. Please retry", error);
      if (held) {
        try { await this.deps.release(); }
        catch (releaseError) { this.say("无法恢复邮件处理，请退出并重新打开 EMA", "Could not resume mail processing. Quit and reopen EMA", releaseError); }
      }
    }
  }
}
