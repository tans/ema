import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { ArrowLeft, CheckCircle, Eye, EyeOff, Filter, RefreshCw, Search, Settings as SettingsIcon } from "react-feather";
import "daisyui/theme/light.css";
import "daisyui/components/button.css";
import "daisyui/components/input.css";
import "./style.css";
import MailHistory from "./MailHistory";
import ToastHost, { notify } from "./Toast";
import UpdateSettings, { useAppUpdater } from "./UpdateSettings";

type MailSettings = {
  provider: string; address: string; username: string;
  imap_host: string; imap_port: number; smtp_host: string; smtp_port: number;
};
type AgentRoute = { agent: string; prefix: string; enabled: boolean };
type Settings = {
  language: string; mail: MailSettings;
  router: { base_url: string; model: string }; agent_routes: AgentRoute[];
  allowlist: string[]; projects: { alias: string; path: string }[];
  codex_command: string; workspace_write: boolean;
  poll_seconds: number; progress_minutes: number[];
  enabled: boolean; onboarding_completed: boolean;
};
type SettingsTab = "mail" | "allowlist" | "agents" | "updates";
const agentNames = ["codex", "cc", "opencode", "pi", "omp", "zcode", "kimi", "dsh", "hermes", "openclaw", "gemini"];
const qqMail = (address: string): MailSettings => ({
  provider: "qq", address, username: address, imap_host: "imap.qq.com",
  imap_port: 993, smtp_host: "smtp.qq.com", smtp_port: 465
});
const initial: Settings = {
  language: "zh-CN", mail: qqMail(""), router: { base_url: "https://api.openai.com/v1", model: "" },
  agent_routes: agentNames.map(agent => ({ agent, prefix: agent, enabled: true })),
  allowlist: [], projects: [], codex_command: "codex", workspace_write: false,
  poll_seconds: 30, progress_minutes: [10, 30, 60], enabled: false, onboarding_completed: false
};
const labels = {
  "zh-CN": {
    search: "搜索邮件", refresh: "刷新邮件", settings: "设置", back: "返回邮件", loginMail: "登录邮箱", language: "语言",
    mailbox: "邮箱", senders: "允许发信的邮箱", agents: "Agent",
    qq: "QQ 邮箱", other: "其他邮箱", address: "邮箱地址", username: "登录用户名",
    imap: "IMAP 服务器", smtp: "SMTP 服务器", port: "端口", auth: "授权码", password: "应用密码",
    secretPlaceholder: "输入邮箱授权码或应用密码", keepSecret: "已保存，留空保持现有凭据",
    showSecret: "显示授权码", hideSecret: "隐藏授权码", getCode: "获取 QQ 邮箱授权码",
    secretStorage: "凭据以明文存于本机 .ema/secrets.json；macOS 和 Linux 文件仅当前用户可读写。",
    testConnection: "测试连接", checking: "正在验证…", connected: "IMAP 连接成功",
    save: "保存设置", saving: "保存中…", saved: "设置已保存", error: "操作失败",
    missingEmail: "请填写邮箱地址和授权码。", missingSecret: "请补充邮箱凭据后登录。",
    senderHelp: "一行一个邮箱。只有这些地址发来的邮件可以创建或继续任务。",
    enabled: "接收邮件任务", enabledHint: "先保存邮箱和允许发信的地址，再启用邮件任务。",
    prefixes: "邮件主题前缀", prefixHelp: "当前支持 Codex、Claude Code、Gemini 执行；其他 Agent 会回复不支持的原因。",
    hookNotice: "本地 Agent 报告会发送到当前邮箱，可能包含会话问题、上下文和结果。",
    testMail: "发送测试邮件", recipient: "测试收件人", sending: "正在发送…",
    sent: "SMTP 已接受测试邮件，请检查收件箱。", testHint: "向已保存的白名单地址发信；SMTP 接受不代表最终送达。",
    saveFirst: "请先保存当前设置，再发送测试邮件。", invalidReceiver: "请选择已保存的白名单收件人。",
    filter: "过滤邮件类型", filterKinds: "显示哪些类型的邮件？", filterHint: "默认隐藏回环和无关邮件。",
    kindLabels: {
      echo: "回环", unrelated: "无关", new_task: "新任务", resume_task: "继续任务", invalid: "已拒绝",
      new_task_ack: "新任务回执", resume_ack: "继续回执", progress: "进度", result: "结果", error: "错误", hook: "Hook", test: "测试", legacy: "历史"
    }
  },
  en: {
    search: "Search mail", refresh: "Refresh mail", settings: "Settings", back: "Back to mail", loginMail: "Connect mailbox", language: "Language",
    mailbox: "Mailbox", senders: "Allowed senders", agents: "Agents",
    qq: "QQ Mail", other: "Other email", address: "Email address", username: "Login username",
    imap: "IMAP server", smtp: "SMTP server", port: "Port", auth: "Authorization code", password: "App password",
    secretPlaceholder: "Enter your authorization code or app password", keepSecret: "Saved; leave blank to keep it",
    showSecret: "Show password", hideSecret: "Hide password", getCode: "Get QQ Mail authorization code",
    secretStorage: "Credentials are stored as plain text in local .ema/secrets.json; the file is owner-only on macOS and Linux.",
    testConnection: "Test connection", checking: "Checking…", connected: "IMAP connection successful",
    save: "Save settings", saving: "Saving…", saved: "Settings saved", error: "Action failed",
    missingEmail: "Enter your mailbox address and app password.", missingSecret: "Enter your mailbox password to connect.",
    senderHelp: "One address per line. Only these senders can create or continue tasks.",
    enabled: "Receive email tasks", enabledHint: "Save a mailbox and allowed senders before enabling email tasks.",
    prefixes: "Subject prefixes", prefixHelp: "Codex, Claude Code and Gemini run; other Agents receive an explanatory reply.",
    hookNotice: "Local Agent reports are emailed to this mailbox and may include questions, context and results.",
    testMail: "Send test email", recipient: "Test recipient", sending: "Sending…",
    sent: "SMTP accepted the test email. Check your inbox.", testHint: "Sends to a saved allowed sender. SMTP acceptance does not guarantee delivery.",
    saveFirst: "Save the current settings before sending a test email.", invalidReceiver: "Choose a saved allowed sender.",
    filter: "Filter mail types", filterKinds: "Which mail types to show?", filterHint: "Echo and unrelated mail are hidden by default.",
    kindLabels: {
      echo: "Echo", unrelated: "Unrelated", new_task: "New task", resume_task: "Resume", invalid: "Rejected",
      new_task_ack: "Task ack", resume_ack: "Resume ack", progress: "Progress", result: "Result", error: "Error", hook: "Hook", test: "Test", legacy: "Legacy"
    }
  }
};
const normalizeAllowlist = (raw: string) => Array.from(new Set(raw.split(/[\n,]/).map(s => s.trim().toLowerCase()).filter(Boolean)));
// Mail history kind filter. Echo and unrelated mail are hidden by default.
const ALL_KINDS = ["echo", "unrelated", "new_task", "resume_task", "invalid", "new_task_ack", "resume_ack", "progress", "result", "error", "hook", "test", "legacy"] as const;
const DEFAULT_HIDDEN: Record<string, boolean> = { echo: true, unrelated: true };

export default function App() {
  const [settings, setSettings] = useState(initial);
  const [savedSettings, setSavedSettings] = useState(initial);
  const [view, setView] = useState<"history" | "settings">("history");
  const [search, setSearch] = useState("");
  const [version, setVersion] = useState(0);
  const [receiving, setReceiving] = useState(false);
  const setReceiveNotice = (text: string) => notify(text, text.startsWith("收件失败") || text.startsWith("Receive failed") ? "error" : "info");
  async function receiveMail() {
    if (receiving) return;
    setReceiving(true); setReceiveNotice("");
    try {
      await invoke("receive_mail_now");
      setReceiveNotice(language === "zh-CN" ? "收件完成，邮件记录已刷新。" : "Mail checked. History refreshed.");
    } catch (e) {
      setReceiveNotice(`${language === "zh-CN" ? "收件失败" : "Receive failed"}: ${e}`);
    } finally {
      setVersion(v => v + 1);
      setReceiving(false);
    }
  }
  const [settingsTab, setSettingsTab] = useState<SettingsTab>("mail");
  const [loading, setLoading] = useState(true);
  const [mailboxReady, setMailboxReady] = useState(false);
  const [secret, setSecret] = useState("");
  const [showSecret, setShowSecret] = useState(false);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [sending, setSending] = useState(false);
  const [connectionOk, setConnectionOk] = useState(false);
  const [testRecipient, setTestRecipient] = useState("");
  const [allowlistText, setAllowlistText] = useState("");
  const setNotice = (text: string) => notify(text, text.startsWith(labels[settings.language === "en" ? "en" : "zh-CN"].error) ? "error" : "info");
  const [filterOpen, setFilterOpen] = useState(false);
  const [hiddenKinds, setHiddenKinds] = useState<Record<string, boolean>>({ ...DEFAULT_HIDDEN });
  const [loadError, setLoadError] = useState("");
  const anyHidden = Object.values(hiddenKinds).some(Boolean);
  const language = settings.language === "en" ? "en" : "zh-CN";
  const t = labels[language];
  const updateState = useAppUpdater(language, !loading);
  const updateLabel = language === "zh-CN" ? "软件更新" : "Updates";
  const busy = saving || testing || sending;
  const dirty = JSON.stringify(settings) !== JSON.stringify(savedSettings) || !!secret.trim();
  const selected = settings.mail.provider === "qq" ? "qq" : "custom";

  useEffect(() => {
    Promise.all([invoke<Settings>("get_settings"), invoke<boolean>("has_mail_secret")])
      .then(([value, hasSecret]) => {
        const resolved = { ...value, mail: value.mail.address || value.mail.imap_host ? value.mail : qqMail("") };
        setSettings(resolved); setSavedSettings(resolved);
        setAllowlistText(resolved.allowlist.join("\n"));
        setTestRecipient(resolved.allowlist[0] ?? "");
        setMailboxReady(!!resolved.mail.address.trim() && hasSecret);
      }).catch(e => {
        console.error("Could not load local settings", e);
        setLoadError("无法读取本地设置，请重新启动 EMA。 / Restart EMA to load local settings.");
        notify("无法读取本地设置，请重新启动 EMA。 / Restart EMA to load local settings.", "error");
      }).finally(() => setLoading(false));
  }, []);

  function patch(next: Partial<Settings>) { setSettings(s => ({ ...s, ...next })); }
  function patchMail(next: Partial<MailSettings>) {
    setConnectionOk(false);
    setSettings(s => ({ ...s, mail: { ...s.mail, ...next } }));
  }
  function openSettings() { setSettingsTab("mail"); setNotice(""); setView("settings"); }
  function back() {
    setSettings(savedSettings); setAllowlistText(savedSettings.allowlist.join("\n"));
    setSecret(""); setShowSecret(false); setConnectionOk(false); setNotice(""); setView("history");
  }
  async function validateConnection() {
    if (!settings.mail.address.trim()) { setNotice(t.missingEmail); return false; }
    setTesting(true); setNotice("");
    try {
      await invoke("check_mail", { settings, password: secret.trim() || null });
      setConnectionOk(true); setNotice(t.connected); return true;
    } catch (e) { setConnectionOk(false); setNotice(`${t.error}: ${e}`); return false; }
    finally { setTesting(false); }
  }
  async function save() {
    const connecting = !mailboxReady;
    if (connecting) setSettingsTab("mail");
    if ((connecting || JSON.stringify(settings.mail) !== JSON.stringify(savedSettings.mail) || secret.trim())
        && !(await validateConnection())) return;
    setSaving(true); setNotice("");
    try {
      const next = { ...settings, onboarding_completed: true };
      if (secret.trim()) await invoke("set_secret", { name: "mail", value: secret.trim() });
      await invoke("save_settings", { settings: next });
      const hasSecret = await invoke<boolean>("has_mail_secret");
      setMailboxReady(!!next.mail.address.trim() && hasSecret);
      setSettings(next); setSavedSettings(next); setSecret(""); setLoadError("");
      setTestRecipient(next.allowlist[0] ?? "");
      if (connecting && hasSecret) setView("history");
      else setNotice(t.saved);
    } catch (e) { setNotice(`${t.error}: ${e}`); }
    finally { setSaving(false); }
  }
  async function sendTest() {
    if (dirty) { setNotice(t.saveFirst); return; }
    if (!savedSettings.allowlist.includes(testRecipient)) { setNotice(t.invalidReceiver); return; }
    setSending(true); setNotice("");
    try { await invoke("send_test_mail", { recipient: testRecipient }); setNotice(t.sent); }
    catch (e) { setNotice(`${t.error}: ${e}`); }
    finally { setSending(false); }
  }

  if (loading) return <main className="app loading" aria-busy="true">EMA</main>;
  return <main className={`app app-${view}`}>
    <ToastHost language={language}/>
    <header className="header">
      {view === "settings" ? <button type="button" className="btn btn-ghost back-button" disabled={busy} onClick={back}>
        <ArrowLeft size={18} aria-hidden="true"/>{t.back}
      </button> : <div className="brand"><img src="/ema-logo.svg" alt=""/>EMA</div>}
      {view === "history" && mailboxReady && <label className="header-search"><Search size={14} aria-hidden="true"/>
        <input className="input input-ghost" type="search" aria-label={t.search} placeholder={t.search} value={search} onChange={e => setSearch(e.target.value)}/>
        <button type="button" className="btn btn-ghost btn-square refresh-button" aria-label={receiving ? (language === "zh-CN" ? "正在收件…" : "Receiving…") : t.refresh} title={t.refresh} disabled={receiving} aria-busy={receiving} onClick={receiveMail}><RefreshCw size={14} aria-hidden="true"/></button>
      </label>}
      {view === "settings" ? <select className="language" aria-label={t.language} value={language} disabled={busy}
        onChange={e => patch({ language: e.target.value })}><option value="zh-CN">中文</option><option value="en">English</option></select>
        : <div className="header-actions">
          {mailboxReady && <>
            <div className="filter-wrap">
              <button type="button" className={"btn btn-ghost btn-square filter-button" + (anyHidden ? " filter-active" : "")}
                aria-label={t.filter} title={t.filter} aria-expanded={filterOpen}
                onClick={() => setFilterOpen(open => !open)}><Filter size={15} aria-hidden="true"/></button>
              {filterOpen && <>
                <div className="filter-backdrop" onClick={() => setFilterOpen(false)}/>
                <div className="filter-popover" role="dialog" aria-label={t.filterKinds}>
                  <p className="filter-title">{t.filterKinds}</p>
                  <p className="filter-hint">{t.filterHint}</p>
                  {ALL_KINDS.map(kind => <label className="filter-row" key={kind}>
                    <input type="checkbox" checked={!hiddenKinds[kind]} onChange={() =>
                      setHiddenKinds(hidden => ({ ...hidden, [kind]: !hidden[kind] }))}/>
                    <span>{t.kindLabels[kind] ?? kind}</span>
                  </label>)}
                </div>
              </>}
            </div>
          </>}
          <button type="button" className="btn btn-ghost btn-square settings-button" aria-label={updateState.version ? `${t.settings} · ${updateLabel} ${updateState.version}` : t.settings} title={updateState.version ? `${updateLabel} ${updateState.version}` : t.settings} onClick={openSettings}>
            <SettingsIcon size={19} aria-hidden="true"/>{updateState.version && <span aria-hidden="true">•</span>}
          </button>
        </div>}
    </header>
    {view === "history" ? mailboxReady ? <div className="app-content"><MailHistory language={language} search={search} version={version} hiddenKinds={hiddenKinds}/></div>
      : <div className="login-gate"><button type="button" className="btn login-button" onClick={openSettings}>{t.loginMail}</button>
      </div>
      : <div className="settings-page">
        <h1>{t.settings}</h1>
        <nav className="settings-nav" aria-label={t.settings}>
          {([['mail', t.mailbox], ['allowlist', t.senders], ['agents', t.agents], ['updates', updateLabel + (updateState.version ? ' •' : '')]] as const).map(([id, label]) =>
            <button type="button" key={id} className={`btn btn-ghost settings-nav-item ${settingsTab === id ? 'active' : ''}`}
              aria-current={settingsTab === id ? "page" : undefined} disabled={busy} onClick={() => { setSettingsTab(id); setNotice(""); }}>{label}</button>)}
        </nav>
        {settingsTab === "updates" && <UpdateSettings language={language} blocked={busy || dirty}/>}
        <fieldset className="settings-fields" disabled={busy}>
          {settingsTab === "mail" && <>
            <div className="choice">
              <button type="button" className={`btn btn-ghost ${selected === 'qq' ? 'selected' : ''}`} aria-pressed={selected === 'qq'}
                onClick={() => patchMail(qqMail(settings.mail.address))}>{t.qq}</button>
              <button type="button" className={`btn btn-ghost ${selected === 'custom' ? 'selected' : ''}`} aria-pressed={selected === 'custom'}
                onClick={() => patchMail({ provider: "custom" })}>{t.other}</button>
            </div>
            <label className="field"><span>{t.address}</span><input className="input input-ghost" type="email" autoComplete="email" placeholder="name@example.com"
              value={settings.mail.address} onChange={e => patchMail(selected === 'qq' ? { address: e.target.value, username: e.target.value } : { address: e.target.value })}/></label>
            {selected === 'custom' && <>
              <label className="field"><span>{t.username}</span><input className="input input-ghost" value={settings.mail.username} onChange={e => patchMail({ username: e.target.value })}/></label>
              <div className="servers">
                <label className="field"><span>{t.imap}</span><input className="input input-ghost" value={settings.mail.imap_host} onChange={e => patchMail({ imap_host: e.target.value })}/></label>
                <label className="field"><span>{t.port}</span><input className="input input-ghost" type="number" min="1" max="65535" value={settings.mail.imap_port} onChange={e => patchMail({ imap_port: Number(e.target.value) })}/></label>
                <label className="field"><span>{t.smtp}</span><input className="input input-ghost" value={settings.mail.smtp_host} onChange={e => patchMail({ smtp_host: e.target.value })}/></label>
                <label className="field"><span>{t.port}</span><input className="input input-ghost" type="number" min="1" max="65535" value={settings.mail.smtp_port} onChange={e => patchMail({ smtp_port: Number(e.target.value) })}/></label>
              </div>
            </>}
            <label className="field"><span>{selected === 'qq' ? t.auth : t.password}</span>
              <div className="secret-input"><input className="input input-ghost" type={showSecret ? 'text' : 'password'} autoComplete="new-password" value={secret}
                placeholder={mailboxReady ? t.keepSecret : t.secretPlaceholder} onChange={e => { setSecret(e.target.value); setConnectionOk(false); }}/>
                <button type="button" className="btn btn-ghost btn-square secret-toggle" aria-label={showSecret ? t.hideSecret : t.showSecret}
                  onClick={() => setShowSecret(!showSecret)}>{showSecret ? <EyeOff size={16}/> : <Eye size={16}/>}</button>
              </div>
            </label>
            <div className="inline-actions">
              {selected === 'qq' && <a href="https://help.mail.qq.com/detail/106/985" target="_blank" rel="noreferrer">{t.getCode}</a>}
              <button type="button" className="btn btn-ghost" onClick={() => { void validateConnection(); }}>{testing ? t.checking : t.testConnection}</button>
            </div>
            {connectionOk && <p className="success-text"><CheckCircle size={15}/>{t.connected}</p>}
            <p className="field-hint">{t.secretStorage}</p>
            <p className="field-hint">{t.hookNotice}</p>
            {!mailboxReady && savedSettings.mail.address && <p className="field-hint">{t.missingSecret}</p>}
            {mailboxReady && <details className="advanced"><summary>{t.testMail}</summary>
              <label className="field"><span>{t.recipient}</span><select value={testRecipient} onChange={e => setTestRecipient(e.target.value)}>
                <option value="">—</option>{savedSettings.allowlist.map(address => <option key={address}>{address}</option>)}
              </select></label>
              <button type="button" className="btn btn-ghost" disabled={dirty || !testRecipient} onClick={() => { void sendTest(); }}>{sending ? t.sending : t.testMail}</button>
              <p className="field-hint">{dirty ? t.saveFirst : t.testHint}</p>
            </details>}
          </>}
          {settingsTab === "allowlist" && <>
            <label className="field"><span>{t.senders}</span><textarea rows={5} value={allowlistText} placeholder="you@example.com"
              onChange={e => { setAllowlistText(e.target.value); patch({ allowlist: normalizeAllowlist(e.target.value) }); }}/></label>
            <p className="field-hint">{t.senderHelp}</p>
          </>}
          {settingsTab === "agents" && <>
            <label className="switch-line"><span>{t.enabled}</span><input type="checkbox" role="switch" checked={settings.enabled}
              disabled={!savedSettings.allowlist.length || !mailboxReady} onChange={e => patch({ enabled: e.target.checked })}/></label>
            <p className="field-hint">{t.enabledHint}</p>
            <details className="advanced"><summary>{t.prefixes}</summary><div className="routes">
              {settings.agent_routes.map((route, index) => <label className="route-row" key={route.agent}><span>{route.agent}</span>
                <input className="input input-ghost" aria-label={`${route.agent} prefix`} value={route.prefix} onChange={e => patch({ agent_routes: settings.agent_routes.map((a, i) => i === index ? { ...a, prefix: e.target.value, enabled: true } : a) })}/>
              </label>)}
            </div><p className="field-hint">{t.prefixHelp}</p></details>
          </>}
        </fieldset>
        {settingsTab !== "updates" && <footer className="settings-footer">

          <button type="button" className="btn primary-button" disabled={busy} onClick={() => { void save(); }}>
            {saving ? t.saving : testing ? t.checking : !mailboxReady ? t.loginMail : t.save}
          </button>
        </footer>}
      </div>}
  </main>;
}
