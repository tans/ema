import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import {
  ArrowLeft, ArrowRight, Check, CheckCircle2, ChevronDown, CircleHelp,
  Mail, Settings2, ShieldCheck, Send, Activity
} from "lucide-react";
import "./style.css";
import { Archive } from "lucide-react";
import MailHistory from "./MailHistory";

type MailSettings = {
  provider: string; address: string; username: string;
  imap_host: string; imap_port: number; smtp_host: string; smtp_port: number;
};
type AgentRoute = { agent: string; prefix: string; enabled: boolean };
type Settings = {
  language: string; mail: MailSettings;
  router: { base_url: string; model: string };
  agent_routes: AgentRoute[];
  allowlist: string[]; projects: { alias: string; path: string }[];
  codex_command: string; workspace_write: boolean;
  poll_seconds: number; progress_minutes: number[];
  enabled: boolean; onboarding_completed: boolean;
};
type Stats = {
  total_sessions: number; pending_jobs: number; running_jobs: number;
  hook_reports: number; hook_sent: number; hook_pending: number;
  completed_jobs: number; failed_jobs: number; received_messages: number;
  last_activity: number | null;
};
type View = "welcome" | "dashboard" | "settings" | "history";
type Step = 1 | 2 | 3;
type SettingsTab = "mail" | "allowlist" | "agents";

const agentNames = ["codex", "cc", "opencode", "pi", "omp", "zcode", "kimi", "dsh", "hermes", "openclaw", "gemini"];
const qqMail = (address: string): MailSettings => ({
  provider: "qq", address, username: address, imap_host: "imap.qq.com",
  imap_port: 993, smtp_host: "smtp.qq.com", smtp_port: 465
});
const initial: Settings = {
  language: "zh-CN", mail: qqMail(""),
  router: { base_url: "https://api.openai.com/v1", model: "" },
  agent_routes: agentNames.map(agent => ({ agent, prefix: agent, enabled: true })),
  allowlist: [], projects: [], codex_command: "codex", workspace_write: false,
  poll_seconds: 30, progress_minutes: [10, 30, 60],
  enabled: false, onboarding_completed: false
};

const labels = {
  "zh-CN": {
    language: "语言", skipAll: "跳过引导", skipStep: "跳过此步", back: "上一步",
    settings: "设置", home: "运行概览", history: "邮件记录", setup: "初始设置", step: "步骤",
    emailStep: "连接邮箱", allowStep: "设置白名单", testStep: "发送测试邮件",
    emailDesc: "连接日常邮箱，授权码会安全保存在系统凭据中。",
    allowDesc: "只有下列邮箱地址发来的任务才允许进入 Agent。",
    testDesc: "向白名单邮箱发送一封测试邮件，确认 SMTP 发信正常。",
    qq: "QQ 邮箱", other: "其他邮箱", email: "邮箱", address: "邮箱地址",
    login: "登录用户名", auth: "授权码", password: "应用密码",
    authPlaceholder: "输入 QQ 邮箱授权码", keepSecret: "留空则使用已保存的凭据",
    imap: "IMAP 服务器", smtp: "SMTP 服务器", port: "端口",
    getCode: "获取授权码", checkConnection: "测试连接",
    checking: "正在验证…", connected: "IMAP 连接成功",
    next: "保存并继续", allowSave: "保存白名单并继续",
    senders: "允许发信的邮箱", senderHelp: "一行一个邮箱地址，可添加多个。",
    chooseReceiver: "测试收件人", sendTest: "发送测试邮件",
    sending: "正在发送…", sent: "SMTP 已接受测试邮件，请检查收件箱。",
    testNoDelivery: "发信成功不代表邮件一定投递或已被收到。",
    finish: "完成", finishEnable: "完成并启用 EMA", finishSkip: "完成但暂不启用",
    running: "已启用", paused: "未启用",
    runningDesc: "正在监控符合白名单与主题前缀规则的新邮件",
    pausedDesc: "邮件任务已暂停；本机 Agent 完成后仍会自动邮件汇报",
    overviewStats: "会话统计", sessions: "总会话", active: "执行中", queued: "排队中",
    completed: "已完成", failed: "失败 / 中断", received: "任务邮件",
    hookHeading: "本机会话汇报", hooksCaptured: "已捕获", hooksSent: "已发送", hooksWaiting: "待发送",
    lastActivity: "最近活动", none: "暂无活动",
    mailboxConfig: "邮箱连接", agent: "本地 Agent", hookNotice: "启动 EMA 后自动接入已安装的本机 Agent Hook，任务完成时汇报到当前邮箱。邮件可能包含会话标题、用户问题、近期对话和执行结果，请注意其中的敏感信息。",
    unavailable: "未检测到", available: "可用",
    routes: "邮件主题前缀", notSupported: "全部前缀默认生效。当前支持 Codex、Claude Code、Gemini 执行，其他 Agent 会自动邮件回复不支持的原因。",
    enabled: "启用邮件 Agent",
    save: "保存设置", saving: "保存中…",
    saved: "设置已保存", error: "操作失败",
    missingEmail: "请先填写邮箱地址和授权码并验证连接",
    missingAllow: "请添加至少一个白名单邮箱，或跳过此步",
    invalidReceiver: "请选择已保存的白名单收件人",
    reopenGuide: "重新打开引导", refresh: "刷新 Agent 检测",
    continue: "继续", enabledHint: "须正确配置邮箱和白名单后才能启用。",
    ready: "配置已完成", secure: "仅允许白名单和主题前缀匹配的任务"
  },
  en: {
    language: "Language", skipAll: "Skip setup", skipStep: "Skip this step", back: "Back",
    settings: "Settings", home: "Overview", history: "Mail history", setup: "Get started", step: "Step",
    emailStep: "Connect email", allowStep: "Sender allowlist", testStep: "Send a test email",
    emailDesc: "Connect your everyday mailbox. Your app password stays in your OS keychain.",
    allowDesc: "Only these email addresses may submit tasks to an Agent.",
    testDesc: "Send a real SMTP test email to an allowed address.",
    qq: "QQ Mail", other: "Other email", email: "Mailbox", address: "Email address",
    login: "Login username", auth: "Authorization code", password: "App password",
    authPlaceholder: "Enter QQ Mail authorization code", keepSecret: "Leave blank to use the stored credential",
    imap: "IMAP server", smtp: "SMTP server", port: "Port",
    getCode: "Get authorization code", checkConnection: "Test connection",
    checking: "Checking…", connected: "IMAP connection successful",
    next: "Save and continue", allowSave: "Save allowlist and continue",
    senders: "Allowed senders", senderHelp: "One address per line. Multiple addresses are supported.",
    chooseReceiver: "Test recipient", sendTest: "Send test email",
    sending: "Sending…", sent: "SMTP accepted the test email. Check your inbox.",
    testNoDelivery: "SMTP acceptance does not guarantee delivery or receipt.",
    finish: "Finish", finishEnable: "Finish and enable EMA", finishSkip: "Finish without enabling",
    running: "Enabled", paused: "Disabled",
    runningDesc: "Monitoring new emails with allowed senders and agent prefixes",
    pausedDesc: "Inbound tasks are paused; local Agent completion reports remain automatic",
    overviewStats: "Session statistics", sessions: "Sessions", active: "Running", queued: "Queued",
    completed: "Completed", failed: "Failed / interrupted", received: "Task emails",
    hookHeading: "Local Agent reports", hooksCaptured: "Captured", hooksSent: "Sent", hooksWaiting: "Pending",
    lastActivity: "Last activity", none: "No activity yet",
    mailboxConfig: "Email connection", agent: "Local Agents", hookNotice: "EMA automatically attaches local Agent completion hooks and emails reports to this mailbox while running. Reports may contain sensitive session titles, questions, recent conversation and results.",
    unavailable: "Not found", available: "Available",
    routes: "Subject prefixes", notSupported: "All prefixes are active. Codex, Claude Code and Gemini run; other agents receive an explanatory email.",
    enabled: "Enable email Agent",
    save: "Save settings", saving: "Saving…",
    saved: "Settings saved", error: "Action failed",
    missingEmail: "Enter your mailbox and validate the connection",
    missingAllow: "Add at least one allowed sender or skip this step",
    invalidReceiver: "Choose a saved allowlist recipient",
    reopenGuide: "Open setup guide", refresh: "Scan Agents",
    continue: "Continue", enabledHint: "A mailbox and allowed sender are required to enable the gateway.",
    ready: "Setup complete", secure: "Only allowed senders with matching prefixes can start tasks"
  }
};

function normalizeAllowlist(raw: string): string[] {
  return Array.from(new Set(raw.split(/[\n,]/).map(s => s.trim().toLowerCase()).filter(Boolean)));
}

export default function App() {
  const [settings, setSettings] = useState<Settings>(initial);
  const [view, setView] = useState<View>("welcome");
  const [step, setStep] = useState<Step>(1);
  const [settingsTab, setSettingsTab] = useState<SettingsTab>("mail");
  const [loading, setLoading] = useState(true);
  const [secret, setSecret] = useState("");
  const [stats, setStats] = useState<Stats | null>(null);
  const [routesOpen, setRoutesOpen] = useState(false);

  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [sending, setSending] = useState(false);
  const [connectionOk, setConnectionOk] = useState(false);
  const [testSent, setTestSent] = useState(false);
  const [testRecipient, setTestRecipient] = useState("");
  const [allowlistText, setAllowlistText] = useState("");
  const [notice, setNotice] = useState("");
  const language = settings.language === "en" ? "en" : "zh-CN";
  const t = labels[language];
  const selected = settings.mail.provider === "qq" ? "qq" : "custom";

  function patch(next: Partial<Settings>) { setSettings(s => ({ ...s, ...next })); }
  function patchMail(next: Partial<MailSettings>) {
    setConnectionOk(false);
    setTestSent(false);
    setSettings(s => ({ ...s, mail: { ...s.mail, ...next } }));
  }
  function changeProvider(value: string) {
    setNotice("");
    patchMail(value === "qq" ? qqMail(settings.mail.address) : { provider: "custom" });
  }
  function changeAddress(value: string) {
    patchMail(selected === "qq" ? { address: value, username: value } : { address: value });
  }

  async function refreshStats() {
    try { setStats(await invoke<Stats>("get_stats")); }
    catch { /* Preserve previous metrics while disconnected. */ }
  }
  useEffect(() => {
    invoke<Settings>("get_settings").then(value => {
      const configured = value.mail.address || value.mail.imap_host;
      const resolved = { ...value, mail: configured ? value.mail : qqMail("") };
      setSettings(resolved);
      setTestRecipient(resolved.allowlist[0] ?? "");
      setAllowlistText(resolved.allowlist.join("\n"));
      setView(resolved.onboarding_completed ? "dashboard" : "welcome");
    }).catch(e => setNotice(String(e))).finally(() => setLoading(false));
    void refreshStats();
    const timer = setInterval(() => { void refreshStats(); }, 12000);
    return () => clearInterval(timer);
  }, []);

  async function persist(next: Settings, showSaved = false): Promise<boolean> {
    setSaving(true);
    setNotice("");
    try {
      if (secret.trim()) await invoke("set_secret", { name: "mail", value: secret.trim() });
      await invoke("save_settings", { settings: next });
      setSettings(next);
      setSecret("");
      if (showSaved) setNotice(t.saved);
      return true;
    } catch (e) {
      setNotice(t.error + ": " + String(e));
      return false;
    } finally { setSaving(false); }
  }
  async function validateConnection(): Promise<boolean> {
    if (!settings.mail.address.trim()) { setNotice(t.missingEmail); return false; }
    setTesting(true);
    setNotice("");
    try {
      await invoke("check_mail", { settings, password: secret.trim() || null });
      setConnectionOk(true);
      setNotice(t.connected);
      return true;
    } catch (e) {
      setConnectionOk(false);
      setNotice(t.error + ": " + String(e));
      return false;
    } finally { setTesting(false); }
  }

  async function nextEmail() {
    if (!(await validateConnection())) return;
    const next = { ...settings, enabled: settings.enabled && settings.allowlist.length > 0 };
    if (await persist(next)) { setStep(2); setNotice(""); }
  }
  async function nextAllowlist() {
    if (!settings.allowlist.length) { setNotice(t.missingAllow); return; }
    if (await persist(settings)) {
      setTestRecipient(settings.allowlist[0] ?? "");
      setTestSent(false);
      setStep(3);
    }
  }
  async function sendTest() {
    if (!settings.allowlist.some(a => a.toLowerCase() === testRecipient.trim().toLowerCase())) {
      setNotice(t.invalidReceiver); return;
    }
    setSending(true);
    setNotice("");
    try {
      await invoke("send_test_mail", { recipient: testRecipient.trim() });
      setTestSent(true);
      setNotice(t.sent);
    } catch (e) {
      setTestSent(false);
      setNotice(t.error + ": " + String(e));
    } finally { setSending(false); }
  }
  async function finishGuide(enable = false) {
    const canEnable = enable && testSent && connectionOk && !!settings.mail.address && settings.allowlist.length > 0;
    const next = { ...settings, onboarding_completed: true, enabled: canEnable || settings.enabled };
    if (await persist(next)) { setView("dashboard"); setNotice(""); void refreshStats(); }
  }
  function skipStep() {
    setNotice("");
    if (step < 3) {
      if (step === 2) setTestRecipient(settings.allowlist[0] ?? "");
      setStep((step + 1) as Step);
    } else void finishGuide(false);
  }
  async function goHome() {
    try {
      const saved = await invoke<Settings>("get_settings");
      setSettings(saved);
      setAllowlistText(saved.allowlist.join("\n"));
    } catch (e) { setNotice(String(e)); }
    setSecret("");
    setNotice("");
    setView("dashboard");
    void refreshStats();
  }
  function openGuide() {
    setStep(1); setConnectionOk(false); setTestSent(false);
    setTestRecipient(settings.allowlist[0] ?? "");
    setNotice(""); setView("welcome");
  }
  const lastActivity = stats?.last_activity
    ? new Date(stats.last_activity * 1000).toLocaleString(language === "en" ? "en-US" : "zh-CN")
    : t.none;

  function mailboxFields() {
    return <div className="form-body">
      <div className="choice">
        <button type="button" className={selected === "qq" ? "selected" : ""} onClick={() => changeProvider("qq")}>{t.qq}</button>
        <button type="button" className={selected === "custom" ? "selected" : ""} onClick={() => changeProvider("custom")}>{t.other}</button>
      </div>
      <label className="field"><span>{t.address}</span>
        <input type="email" autoComplete="email" placeholder="name@qq.com"
          value={settings.mail.address} onChange={e => changeAddress(e.target.value)} /></label>
      {selected === "custom" && <>
        <label className="field"><span>{t.login}</span>
          <input value={settings.mail.username} onChange={e => patchMail({ username: e.target.value })} /></label>
        <div className="servers">
          <label className="field"><span>{t.imap}</span>
            <input value={settings.mail.imap_host} onChange={e => patchMail({ imap_host: e.target.value })} /></label>
          <label className="field port"><span>{t.port}</span>
            <input type="number" value={settings.mail.imap_port}
              onChange={e => patchMail({ imap_port: Number(e.target.value) })} /></label>
          <label className="field"><span>{t.smtp}</span>
            <input value={settings.mail.smtp_host} onChange={e => patchMail({ smtp_host: e.target.value })} /></label>
          <label className="field port"><span>{t.port}</span>
            <input type="number" value={settings.mail.smtp_port}
              onChange={e => patchMail({ smtp_port: Number(e.target.value) })} /></label>
        </div>
      </>}
      <label className="field"><span>{selected === "qq" ? t.auth : t.password}</span>
        <input type="password" autoComplete="new-password" value={secret}
          placeholder={selected === "qq" ? t.authPlaceholder : t.keepSecret}
          onChange={e => { setSecret(e.target.value); setConnectionOk(false); }} /></label>
      <div className="inline-actions">
        {selected === "qq" && <a href="https://help.mail.qq.com/detail/106/985" target="_blank" rel="noreferrer">{t.getCode}</a>}
        <button type="button" className="link-button" disabled={testing}
          onClick={() => { void validateConnection(); }}>
          {testing ? t.checking : t.checkConnection}
        </button>
      </div>
      {connectionOk && <p className="success-text"><CheckCircle2 size={14} /> {t.connected}</p>}
    </div>;
  }

  function whitelistFields() {
    return <div className="form-body">
      <label className="field"><span>{t.senders}</span>
        <textarea rows={4} value={allowlistText}
          placeholder="you@example.com"
          onChange={e => { setAllowlistText(e.target.value); patch({ allowlist: normalizeAllowlist(e.target.value) }); setTestSent(false); }} />
      </label>
      <p className="field-hint"><ShieldCheck size={14} /> {t.senderHelp}</p>
    </div>;
  }

  function guide() {
    const titles = [t.emailStep, t.allowStep, t.testStep];
    const descriptions = [t.emailDesc, t.allowDesc, t.testDesc];
    return <section className="guide">
      <div className="guide-progress" aria-label={t.setup}>
        {titles.map((title, index) => <div className={"guide-progress-item " + (index + 1 <= step ? "current" : "")} key={title}>
          <span className="progress-number">{index + 1 < step ? <Check size={13} /> : index + 1}</span>
          <span>{title}</span>
        </div>)}
      </div>
      <div className="guide-heading"><span className="eyebrow">{t.step} {step} / 3</span>
        <h1>{titles[step - 1]}</h1><p>{descriptions[step - 1]}</p></div>
      {step === 1 && mailboxFields()}
      {step === 2 && whitelistFields()}
      {step === 3 && <div className="form-body">
        <div className="test-illustration"><Send size={26} /><span>{t.testStep}</span></div>
        {settings.allowlist.length ? <label className="field"><span>{t.chooseReceiver}</span>
          <select value={testRecipient} onChange={e => { setTestRecipient(e.target.value); setTestSent(false); }}>
            <option value="">{t.chooseReceiver}</option>
            {settings.allowlist.map(email => <option value={email} key={email}>{email}</option>)}
          </select>
        </label> : <p className="field-hint">{t.missingAllow}</p>}
        <button type="button" className="outline-button" disabled={sending || !testRecipient || !settings.mail.address}
          onClick={() => { void sendTest(); }}>{sending ? t.sending : t.sendTest}</button>
        {testSent && <p className="success-text"><CheckCircle2 size={14} />{t.sent}</p>}
        <p className="field-hint">{t.testNoDelivery}</p>
      </div>}
      {notice && <div className="notice" role="status">{notice}</div>}
      <div className="guide-actions">
        {step > 1 && <button className="text-button" type="button" onClick={() => {setNotice("");setStep((step - 1) as Step);}}>
          <ArrowLeft size={15} />{t.back}</button>}
        <button className="text-button" type="button" disabled={saving || sending || testing} onClick={skipStep}>{t.skipStep}</button>
        {step === 1 && <button className="primary-button" disabled={saving || testing} onClick={() => { void nextEmail(); }}>{t.next}<ArrowRight size={15}/></button>}
        {step === 2 && <button className="primary-button" disabled={saving} onClick={() => { void nextAllowlist(); }}>{t.allowSave}<ArrowRight size={15}/></button>}
        {step === 3 && <button className="primary-button" disabled={saving} onClick={() => {void finishGuide(testSent);}}>
          {testSent && connectionOk ? t.finishEnable : t.finish}<ArrowRight size={15}/></button>}
      </div>
    </section>;
  }

  function dashboard() {
    const metrics = [
      { label: t.sessions, value: stats?.total_sessions ?? 0, prominent: true },
      { label: t.active, value: stats?.running_jobs ?? 0 },
      { label: t.queued, value: stats?.pending_jobs ?? 0 },
      { label: t.completed, value: stats?.completed_jobs ?? 0 },
      { label: t.failed, value: stats?.failed_jobs ?? 0 },
      { label: t.received, value: stats?.received_messages ?? 0 }
    ];
    return <div className="dashboard">
      <div className={"status-card " + (settings.enabled ? "live" : "")}>
        <div className="status-card-top"><Activity size={17}/><span>{t.home}</span></div>
        <div className="status-heading"><span className="status-indicator"/><h1>{settings.enabled ? t.running : t.paused}</h1></div>
        <p>{settings.enabled ? t.runningDesc : t.pausedDesc}</p>
      </div>
      <section className="dashboard-section">
        <div className="section-heading"><h2>{t.overviewStats}</h2></div>
        <div className="metric-grid">
          {metrics.map(metric => <div className={"metric " + (metric.prominent ? "prominent" : "")} key={metric.label}>
            <span>{metric.label}</span><strong>{metric.value}</strong>
          </div>)}
        </div>
      </section>
      <section className="dashboard-section">
        <div className="section-heading"><h2>{t.hookHeading}</h2></div>
        <div className="metric-grid">
          {[
            {label:t.hooksCaptured,value:stats?.hook_reports ?? 0},
            {label:t.hooksSent,value:stats?.hook_sent ?? 0},
            {label:t.hooksWaiting,value:stats?.hook_pending ?? 0}
          ].map(metric => <div className="metric" key={metric.label}>
            <span>{metric.label}</span><strong>{metric.value}</strong>
          </div>)}
        </div>
      </section>
      <div className="dashboard-bottom"><span>{t.lastActivity}</span><strong>{lastActivity}</strong></div>
    </div>;
  }

  function settingsPanel() {
    const tabs: { id: SettingsTab; label: string; icon: typeof Mail }[] = [
      { id: "mail", label: t.mailboxConfig, icon: Mail },
      { id: "allowlist", label: t.senders, icon: ShieldCheck },
      { id: "agents", label: t.agent, icon: Activity }
    ];
    const activeTab = tabs.find(tab => tab.id === settingsTab)!;
    return <div className="settings-page">
      <div className="page-title">
        <div><span className="eyebrow">{t.settings}</span><h1>{activeTab.label}</h1></div>
        <button className="text-button" type="button" onClick={openGuide}><CircleHelp size={15}/>{t.reopenGuide}</button>
      </div>
      <div className="settings-layout">
        <nav className="settings-nav" aria-label={t.settings}>
          {tabs.map(tab => <button key={tab.id} type="button"
            className={"settings-nav-item " + (settingsTab === tab.id ? "active" : "")}
            aria-current={settingsTab === tab.id ? "page" : undefined}
            onClick={() => { setSettingsTab(tab.id); setNotice(""); }}>
            <tab.icon size={16}/><span>{tab.label}</span>
          </button>)}
        </nav>
        <div className="settings-main">
          {settingsTab === "mail" && <section className="section">
            <h2>{t.mailboxConfig}</h2>{mailboxFields()}
            <p className="field-hint">{t.hookNotice}</p>
          </section>}
          {settingsTab === "allowlist" && <section className="section">
            <h2>{t.senders}</h2>{whitelistFields()}
          </section>}
          {settingsTab === "agents" && <section className="section">
            <h2>{t.agent}</h2>
            <button type="button" className="routes-toggle" aria-expanded={routesOpen}
              onClick={() => setRoutesOpen(!routesOpen)}>
              <span>{t.routes}</span>
              <span className="routes-toggle-meta">{settings.agent_routes.length}
                <ChevronDown size={15} className={routesOpen ? "rotate" : ""}/></span>
            </button>
            {routesOpen && <div className="routes">
              {settings.agent_routes.map((route, index) => <label className="route-row" key={route.agent}>
                <span className="route-name">{route.agent}</span>
                <input aria-label={route.agent + " prefix"} value={route.prefix}
                  onChange={e => patch({agent_routes: settings.agent_routes.map((a,i) =>
                    i === index ? {...a, prefix:e.target.value, enabled:true} : a)})}/>
              </label>)}
            </div>}
            <p className="field-hint">{t.notSupported}</p>
          </section>}

          <div className="settings-footer">
            {notice && <div className="notice" role="status">{notice}</div>}
            <button className="primary-button" type="button" disabled={saving}
              onClick={() => { void persist(settings, true); }}>
              {saving ? t.saving : t.save}
            </button>
          </div>
        </div>
      </div>
    </div>;
  }

  if (loading) return <main className="app"><div className="loading">EMA</div></main>;
  return <main className={"app app-" + view}>
    <aside className="app-sidebar">
      <div className="brand"><img className="brand-icon" src="/ema-logo.svg" alt=""/>
        <div className="brand-title"><strong>EMA</strong><span>Email Agent</span></div>
      </div>
      <nav className="main-nav" aria-label={t.settings}>
        <button className={"main-nav-item " + (view === "dashboard" ? "active" : "")}
          type="button" onClick={() => { void goHome(); }}>
          <Activity size={17}/>{t.home}
        </button>
        <button className={"main-nav-item " + (view === "history" ? "active" : "")}
          type="button" onClick={() => { setNotice(""); setView("history"); }}>
          <Archive size={17}/><span>{t.history}</span>
        </button>
        <button className={"main-nav-item " + (view === "settings" ? "active" : "")}
          type="button" onClick={() => { setNotice(""); setView("settings"); }}>
          <Settings2 size={17}/>{t.settings}
        </button>
      </nav>
      <div className="sidebar-bottom">
        <span className={"status-indicator " + (settings.enabled ? "enabled" : "")}/>
        <span>{settings.enabled ? t.running : t.paused}</span>
      </div>
    </aside>
    <div className="app-main">
      <header className="header">
        <div className="mobile-brand"><img src="/ema-logo.svg" alt=""/>EMA</div>
        {view !== "welcome" && <nav className="mobile-nav" aria-label={t.settings}>
          <button type="button" className={view === "dashboard" ? "active" : ""}
            aria-label={t.home} title={t.home} aria-current={view === "dashboard" ? "page" : undefined}
            onClick={() => { void goHome(); }}><Activity size={17}/></button>
          <button type="button" className={view === "history" ? "active" : ""}
            aria-label={t.history} title={t.history} aria-current={view === "history" ? "page" : undefined}
            onClick={() => { setNotice(""); setView("history"); }}><Archive size={17}/></button>
          <button type="button" className={view === "settings" ? "active" : ""}
            aria-label={t.settings} title={t.settings} aria-current={view === "settings" ? "page" : undefined}
            onClick={() => { setNotice(""); setView("settings"); }}><Settings2 size={17}/></button>
        </nav>}
        <div className="header-context">{view === "dashboard" ? t.home : view === "settings" ? t.settings : view === "history" ? t.history : t.setup}</div>
        <div className="header-right">
          {view === "welcome" && <button type="button" className="header-link"
            disabled={saving || sending || testing} onClick={() => { void finishGuide(false); }}>
            {t.skipAll}
          </button>}
          {view === "welcome" && <select aria-label={t.language} className="language"
            value={language} onChange={e => patch({language:e.target.value})}>
            <option value="zh-CN">中文</option><option value="en">English</option>
          </select>}
          <span className={"header-state " + (settings.enabled ? "running" : "")}>
            <span className="state-dot"/>{settings.enabled ? t.running : t.paused}
          </span>
        </div>
      </header>
      <div className="app-content">
        {view === "welcome" ? guide() : view === "settings" ? settingsPanel() : view === "history" ? <MailHistory language={language}/> : dashboard()}
      </div>
    </div>
  </main>;
}
