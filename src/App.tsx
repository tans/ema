import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { ChevronDown, CircleCheck, RotateCw, Settings2 } from "lucide-react";
import "./style.css";

type Mail = {
  provider: string; address: string; username: string;
  imap_host: string; imap_port: number; smtp_host: string; smtp_port: number;
};
type Settings = {
  language: string; mail: Mail;
  router: { base_url: string; model: string };
  agent_routes: AgentRoute[]; allowlist: string[]; projects: { alias: string; path: string }[];
  codex_command: string; workspace_write: boolean;
  poll_seconds: number; progress_minutes: number[];
  enabled: boolean;
};
type AgentRoute = { agent: string; prefix: string; enabled: boolean };
type Agent = { name: string; installed: boolean; supported: boolean };
type Stats = { pending_jobs: number; running_jobs: number; received_messages: number };
const initial: Settings = {
  language: "zh-CN",
  mail: { provider: "qq", address: "", username: "", imap_host: "imap.qq.com",
    imap_port: 993, smtp_host: "smtp.qq.com", smtp_port: 465 },
  router: { base_url: "https://api.openai.com/v1", model: "" },
  agent_routes: [{agent:"codex",prefix:"codex",enabled:true},{agent:"cc",prefix:"cc",enabled:false},{agent:"opencode",prefix:"opencode",enabled:false},{agent:"pi",prefix:"pi",enabled:false},{agent:"omp",prefix:"omp",enabled:false},{agent:"zcode",prefix:"zcode",enabled:false},{agent:"kimi",prefix:"kimi",enabled:false},{agent:"dsh",prefix:"dsh",enabled:false},{agent:"hermes",prefix:"hermes",enabled:false},{agent:"openclaw",prefix:"openclaw",enabled:false},{agent:"gemini",prefix:"gemini",enabled:false}],
  allowlist: [], projects: [], codex_command: "codex",
  workspace_write: false, poll_seconds: 30, progress_minutes: [10, 30, 60],
  enabled: false,
};
const translations = {
  "zh-CN": {
    running:"已开启", paused:"未开启", email:"邮箱", qq:"QQ 邮箱", other:"其他邮箱",
    address:"邮箱地址", auth:"授权码", authPlaceholder:"输入 QQ 邮箱授权码",
    password:"应用密码", savedSecret:"留空则沿用已保存的凭据",
    help:"获取授权码", test:"测试连接", testing:"连接中…",
    tested:"IMAP 连接成功", mailServer:"收发服务器",
    agent:"本地 Agent", check:"重新检测", available:"可用", unavailable:"未检测到",
    pending:"已安装，暂不支持执行", senders:"允许发信的邮箱", sendersPlaceholder:"you@example.com",
    enabled:"启用邮件 Agent", advanced:"高级设置", command:"Codex 命令",
    poll:"收信间隔（秒）", save:"保存设置", saving:"保存中…",
    saved:"已保存", active:"运行中", queue:"排队中",
    imap:"IMAP 服务器", smtp:"SMTP 服务器", port:"端口",
    error:"操作失败", credentials:"请输入邮箱地址和授权码", unsafe:"请至少填写一个允许的发件人",
    memory:"自动识别执行目录 · 默认只读", routes:"邮件主题前缀", notSupported:"其他 Agent 尚无执行适配器"
  },
  en: {
    running:"Enabled", paused:"Disabled", email:"Email", qq:"QQ Mail", other:"Other IMAP/SMTP",
    address:"Email address", auth:"Authorization code", authPlaceholder:"QQ Mail authorization code",
    password:"App password", savedSecret:"Leave blank to keep the saved credential",
    help:"Get authorization code", test:"Test connection", testing:"Connecting…",
    tested:"IMAP connection successful", mailServer:"Mail servers",
    agent:"Local Agent", check:"Scan again", available:"Available", unavailable:"Not found",
    pending:"Detected, runner unavailable", senders:"Allowed senders", sendersPlaceholder:"you@example.com",
    enabled:"Enable email Agent", advanced:"Advanced", command:"Codex executable",
    poll:"Poll interval (seconds)", save:"Save settings", saving:"Saving…",
    saved:"Saved", active:"Running", queue:"Queued",
    imap:"IMAP host", smtp:"SMTP host", port:"Port",
    error:"Operation failed", credentials:"Enter an email address and authorization code", unsafe:"Add at least one allowed sender",
    memory:"Automatic session directory · read-only", routes:"Email subject prefixes", notSupported:"Other agent runners are not yet available"
  }
};
const qqMail = (address: string): Mail => ({
  provider: "qq", address, username: address,
  imap_host: "imap.qq.com", imap_port: 993,
  smtp_host: "smtp.qq.com", smtp_port: 465,
});

export default function App() {
  const [settings, setSettings] = useState<Settings>(initial);
  const [secret, setSecret] = useState("");
  const [agents, setAgents] = useState<Agent[]>([]);
  const [stats, setStats] = useState<Stats | null>(null);
  const [advanced, setAdvanced] = useState(false);
  const [routesOpen, setRoutesOpen] = useState(false);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [notice, setNotice] = useState("");
  const lang = settings.language === "en" ? "en" : "zh-CN";
  const t = translations[lang];

  function patch(p: Partial<Settings>) { setSettings(s => ({ ...s, ...p })); }
  function patchMail(p: Partial<Mail>) {
    setSettings(s => ({ ...s, mail: { ...s.mail, ...p } }));
  }
  function useProvider(value: string) {
    setNotice("");
    if (value === "qq") patchMail(qqMail(settings.mail.address));
    else patchMail({ provider: "custom" });
  }
  function changeAddress(value: string) {
    patchMail(settings.mail.provider === "qq" ? { address: value, username: value } : { address: value });
  }
  async function refreshAgents() {
    try { setAgents(await invoke<Agent[]>("detect_agents")); }
    catch { setAgents([]); }
  }
  useEffect(() => {
    invoke<Settings>("get_settings").then(value => {
      const fresh = !value.mail.address && !value.mail.imap_host;
      setSettings(fresh ? { ...value, mail: qqMail("") } : value);
    }).catch(e => setNotice(String(e)));
    refreshAgents();
    const refresh = () => invoke<Stats>("get_stats").then(setStats).catch(() => {});
    refresh();
    const timer = setInterval(refresh, 12000);
    return () => clearInterval(timer);
  }, []);

  async function save() {
    setNotice("");
    if (settings.enabled && settings.allowlist.length === 0) {
      setNotice(t.unsafe); return;
    }
    setSaving(true);
    try {
      // Only persist newly supplied secrets; the UI never reads credentials back.
      if (secret.trim()) await invoke("set_secret", { name: "mail", value: secret.trim() });
      await invoke("save_settings", { settings });
      setSecret("");
      setNotice(t.saved);
    } catch (e) { setNotice(t.error + ": " + String(e)); }
    finally { setSaving(false); }
  }
  async function testMail() {
    if (!settings.mail.address || (!secret.trim() && !settings.mail.imap_host)) {
      setNotice(t.credentials); return;
    }
    setTesting(true); setNotice("");
    try {
      await invoke("check_mail", { settings, password: secret.trim() || null });
      setNotice(t.tested);
    } catch (e) { setNotice(t.error + ": " + String(e)); }
    finally { setTesting(false); }
  }
  const selected = settings.mail.provider === "qq" ? "qq" : "custom";
  const available = agents.find(a => a.name === "codex");
  return <main className="app">
    <header className="header">
      <div className="brand"><img className="brand-icon" src="/ema-logo.svg" alt="" /><div className="brand-title"><strong>EMA</strong><span>Email Agent</span></div></div>
      <div className="header-right">
        <select aria-label="Language" className="language" value={lang} onChange={e => patch({ language: e.target.value })}>
          <option value="zh-CN">中文</option><option value="en">English</option>
        </select>
        <span className={"state " + (settings.enabled ? "online" : "")}>
          <i /> {settings.enabled ? t.running : t.paused}
        </span>
      </div>
    </header>
    <div className="brand-accent" aria-hidden="true"><span /></div>

    <section className="section">
      <h2>{t.email}</h2>
      <div className="choice">
        <button className={selected === "qq" ? "selected" : ""} onClick={() => useProvider("qq")}>{t.qq}</button>
        <button className={selected === "custom" ? "selected" : ""} onClick={() => useProvider("custom")}>{t.other}</button>
      </div>
      <label className="field"><span>{t.address}</span>
        <input autoComplete="email" type="email" value={settings.mail.address}
          placeholder="name@qq.com" onChange={e => changeAddress(e.target.value)} />
      </label>
      {selected === "custom" && <>
        <label className="field"><span>{t.address === "Email address" ? "Login username" : "登录用户名"}</span>
          <input value={settings.mail.username} onChange={e => patchMail({ username: e.target.value })} />
        </label>
        <div className="servers">
          <label className="field"><span>{t.imap}</span><input value={settings.mail.imap_host}
            onChange={e => patchMail({ imap_host: e.target.value })} /></label>
          <label className="field port"><span>{t.port}</span><input type="number" value={settings.mail.imap_port}
            onChange={e => patchMail({ imap_port: Number(e.target.value) })} /></label>
          <label className="field"><span>{t.smtp}</span><input value={settings.mail.smtp_host}
            onChange={e => patchMail({ smtp_host: e.target.value })} /></label>
          <label className="field port"><span>{t.port}</span><input type="number" value={settings.mail.smtp_port}
            onChange={e => patchMail({ smtp_port: Number(e.target.value) })} /></label>
        </div>
      </>}
      <label className="field"><span>{selected === "qq" ? t.auth : t.password}</span>
        <input autoComplete="new-password" type="password" value={secret}
          placeholder={selected === "qq" ? t.authPlaceholder : t.savedSecret}
          onChange={e => setSecret(e.target.value)} /></label>
      <div className="inline-actions">
        {selected === "qq" && <a href="https://help.mail.qq.com/detail/106/985" target="_blank" rel="noreferrer">{t.help}</a>}
        <button className="link-button" disabled={testing} onClick={testMail}>
          {testing ? t.testing : t.test}
        </button>
      </div>
    </section>

    <section className="section">
      <div className="line-heading"><h2>{t.agent}</h2>
        <button className="icon-action" aria-label={t.check} title={t.check} onClick={refreshAgents}>
          <RotateCw size={14} /></button></div>
      <div className="agent-line">
        <span className="agent-name">Codex</span>
        <span className={available?.installed ? "agent-ok" : "agent-missing"}>
          {available?.installed ? <CircleCheck size={14}/> : null}{available?.installed ? t.available : t.unavailable}
        </span>
      </div>
      <button type="button" className="routes-toggle" aria-expanded={routesOpen}
        onClick={() => setRoutesOpen(!routesOpen)}>
        <span>{t.routes}</span>
        <span className="routes-toggle-meta">
          {(settings.agent_routes ?? initial.agent_routes).filter(r => r.enabled).length}
          <ChevronDown size={14} className={routesOpen ? "rotate" : ""} />
        </span>
      </button>
      {routesOpen && <div className="routes">
        {(settings.agent_routes ?? initial.agent_routes).map((r, index) => {
          const status=agents.find(a=>a.name===r.agent);
          return <div className="route-row" key={r.agent}>
            <span className="route-name">{r.agent}</span>
            <input aria-label={r.agent+" prefix"} value={r.prefix} onChange={e=>patch({agent_routes:(settings.agent_routes ?? initial.agent_routes).map((x,i)=>i===index?{...x,prefix:e.target.value}:x)})}/>
            <input aria-label={r.agent+" enabled"} type="checkbox" checked={r.enabled} disabled={!status?.installed || !status.supported}
              onChange={e=>patch({agent_routes:(settings.agent_routes ?? initial.agent_routes).map((x,i)=>i===index?{...x,enabled:e.target.checked}:x)})}/>
          </div>;
        })}
      </div>}
      {routesOpen && <small className="routes-note">{t.notSupported}</small>}
    </section>

    <section className="section">
      <label className="field"><span>{t.senders}</span>
        <textarea rows={2} value={settings.allowlist.join("\n")}
          placeholder={t.sendersPlaceholder}
          onChange={e => patch({ allowlist: e.target.value.split(/[\n,]/).map(x => x.trim().toLowerCase()).filter(Boolean) })} />
      </label>
    </section>

    <div className="switch-line">
      <span>{t.enabled}</span>
      <input aria-label={t.enabled} type="checkbox" checked={settings.enabled}
        onChange={e => patch({ enabled: e.target.checked })} />
    </div>

    <button className="advanced-trigger" onClick={() => setAdvanced(!advanced)}>
      <Settings2 size={14} /> {t.advanced} <ChevronDown size={14} className={advanced ? "rotate" : ""} />
    </button>
    {advanced && <section className="advanced-content">
      <label className="field"><span>{t.command}</span>
        <input value={settings.codex_command} onChange={e => patch({ codex_command: e.target.value })} /></label>
      <label className="field"><span>{t.poll}</span>
        <input type="number" value={settings.poll_seconds}
          onChange={e => patch({ poll_seconds: Number(e.target.value) })} /></label>
    </section>}
    <button className="save" disabled={saving} onClick={save}>{saving ? t.saving : t.save}</button>
    {notice && <div className="notice" role="status">{notice}</div>}
    <div className="footer">
      <span>{t.active} {stats?.running_jobs ?? 0} · {t.queue} {stats?.pending_jobs ?? 0}</span>
    </div>
  </main>;
}
