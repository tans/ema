import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import { Activity, ChevronDown, CircleCheck, Folder, Mail, Plus, Settings2, ShieldCheck, Terminal, Trash2 } from "lucide-react";

type Project={alias:string;path:string};
type Settings={
  mail:{address:string;username:string;imap_host:string;imap_port:number;smtp_host:string;smtp_port:number};
  router:{base_url:string;model:string};
  allowlist:string[];projects:Project[];
  codex_command:string;workspace_write:boolean;poll_seconds:number;progress_minutes:number[];enabled:boolean;
};
type Stats={pending_jobs:number;running_jobs:number;received_messages:number;last_activity:number|null};
type AgentStatus={installed:boolean;command:string};
const blank:Settings={
  mail:{address:"",username:"",imap_host:"",imap_port:993,smtp_host:"",smtp_port:465},
  router:{base_url:"https://api.openai.com/v1",model:""},allowlist:[],projects:[],
  codex_command:"codex",workspace_write:false,poll_seconds:30,progress_minutes:[10,30,60],enabled:false
};

export default function App(){
  const [settings,setSettings]=useState<Settings>(blank);
  const [stats,setStats]=useState<Stats|null>(null);
  const [agent,setAgent]=useState<AgentStatus|null>(null);
  const [advanced,setAdvanced]=useState(false);
  const [editing,setEditing]=useState<"mail"|"router"|"agent"|"projects"|"access"|null>(null);
  const [secrets,setSecrets]=useState<Record<string,string>>({mail:"",llm:"",access:""});
  const [notice,setNotice]=useState("");
  const [busy,setBusy]=useState(false);
  const patch=(s:Partial<Settings>)=>setSettings(prev=>({...prev,...s}));
  const changeMail=(key:keyof Settings["mail"],value:string|number)=>
    patch({mail:{...settings.mail,[key]:value}});
  const changeRouter=(key:keyof Settings["router"],value:string)=>
    patch({router:{...settings.router,[key]:value}});
  const refresh=async()=>{
    try {
      const [state,summary,installed]=await Promise.all([
        invoke<Settings>("get_settings"),invoke<Stats>("get_stats"),invoke<AgentStatus>("agent_status")
      ]);
      setSettings(state);setStats(summary);setAgent(installed);
    }catch(e){setNotice(String(e));}
  };
  useEffect(()=>{
    refresh();
    const timer=setInterval(()=>{invoke<Stats>("get_stats").then(setStats).catch(()=>{});},10000);
    return ()=>clearInterval(timer);
  },[]);
  const save=async()=>{
    setBusy(true);setNotice("");
    try {
      for(const [key,value] of Object.entries(secrets)){
        if(value.trim()) await invoke("set_secret",{name:key,value});
      }
      await invoke("save_settings",{settings});
      setSecrets({mail:"",llm:"",access:""});
      setEditing(null);
      setNotice("Settings saved");
      await refresh();
    }catch(e){setNotice(String(e));}
    finally{setBusy(false);}
  };
  const addProject=()=>patch({projects:[...settings.projects,{alias:"",path:""}]});
  const setProject=(idx:number,key:keyof Project,value:string)=>patch({projects:settings.projects.map((p,i)=>i===idx?{...p,[key]:value}:p)});
  const setAllowlist=(s:string)=>patch({allowlist:s.split(/[\n,]/).map(x=>x.trim().toLowerCase()).filter(Boolean)});
  const formatLast=stats?.last_activity?
    new Date(stats.last_activity*1000).toLocaleTimeString([], {hour:"2-digit",minute:"2-digit"}):"Never";
  return <main className="window">
    <header className="top">
      <div className="brand"><div className="logo"><Mail size={20}/></div>
        <div><h1>Email Agent</h1><p>Local Super Agent Gateway</p></div></div>
      <span className={"status "+(settings.enabled?"on":"off")}><i/>{settings.enabled?"Running":"Paused"}</span>
    </header>
    <section className="mail-section">
      <div className="small-label">EMAIL ACCOUNT</div>
      <button className="mail-row" onClick={()=>setEditing(editing==="mail"?null:"mail")}>
        <div><strong>{settings.mail.address||"Connect your email"}</strong>
        <p>{settings.mail.imap_host||"IMAP / SMTP setup required"}</p></div>
        <span className="right"><CircleCheck size={17} className={settings.mail.address?"healthy":"muted"}/><ChevronDown size={15}/></span>
      </button>
    </section>
    <section className="rows">
      <ConfigRow icon={<Terminal size={17}/>} label="Default Agent" value={agent?.installed?"Codex · Installed":"Codex · Not detected"} onClick={()=>setEditing(editing==="agent"?null:"agent")}/>
      <ConfigRow icon={<Activity size={17}/>} label="Router Model" value={settings.router.model||"Default project"} onClick={()=>setEditing(editing==="router"?null:"router")}/>
      <ConfigRow icon={<Folder size={17}/>} label="Project Directories" value={settings.projects.length+" authorized"} onClick={()=>setEditing(editing==="projects"?null:"projects")}/>
      <ConfigRow icon={<ShieldCheck size={17}/>} label="Allowed Senders" value={settings.allowlist.length+" addresses"} onClick={()=>setEditing(editing==="access"?null:"access")}/>
    </section>
    {editing&&<section className="editor">
      <div className="editor-title">{({mail:"Email connection",router:"AI routing",agent:"Local Agent",projects:"Authorized projects",access:"Email access"})[editing]}</div>
      {editing==="mail"&&<>
        <Field label="Email address" value={settings.mail.address} onChange={v=>changeMail("address",v)} placeholder="agent@example.com"/>
        <Field label="Login username" value={settings.mail.username} onChange={v=>changeMail("username",v)} placeholder="agent@example.com"/>
        <div className="two">
          <Field label="IMAP host" value={settings.mail.imap_host} onChange={v=>changeMail("imap_host",v)} placeholder="imap.example.com"/>
          <Field label="IMAP port" type="number" value={String(settings.mail.imap_port)} onChange={v=>changeMail("imap_port",Number(v))}/>
          <Field label="SMTP host" value={settings.mail.smtp_host} onChange={v=>changeMail("smtp_host",v)} placeholder="smtp.example.com"/>
          <Field label="SMTP port" type="number" value={String(settings.mail.smtp_port)} onChange={v=>changeMail("smtp_port",Number(v))}/>
        </div>
        <Field label="App password (saved in OS Keychain)" type="password" value={secrets.mail} onChange={v=>setSecrets({...secrets,mail:v})} placeholder="Leave empty to keep existing"/>
      </>}
      {editing==="router"&&<>
        <Field label="OpenAI-compatible endpoint" value={settings.router.base_url} onChange={v=>changeRouter("base_url",v)}/>
        <Field label="Model" value={settings.router.model} onChange={v=>changeRouter("model",v)} placeholder="Optional when using one project"/>
        <Field label="API key (OS Keychain)" type="password" value={secrets.llm} onChange={v=>setSecrets({...secrets,llm:v})} placeholder="Leave empty to keep existing"/>
      </>}
      {editing==="agent"&&<>
        <Field label="Codex CLI command" value={settings.codex_command} onChange={v=>patch({codex_command:v})}/>
        <p className="hint">The CLI manages its own model credentials, Skills and MCP. EMA does not edit its configuration.</p>
      </>}
      {editing==="projects"&&<>
        {settings.projects.map((p,i)=><div className="project" key={i}>
          <Field label="Alias" value={p.alias} onChange={v=>setProject(i,"alias",v)} placeholder="mfggo"/>
          <Field label="Absolute directory" value={p.path} onChange={v=>setProject(i,"path",v)} placeholder="/workspace/mfggo"/>
          <button className="iconbutton" title="Remove project" onClick={()=>patch({projects:settings.projects.filter((_,j)=>i!==j)})}><Trash2 size={15}/></button>
        </div>)}
        <button className="outline full" onClick={addProject}><Plus size={14}/> Add project</button>
      </>}
      {editing==="access"&&<>
        <label className="field"><span>Allowed addresses (one per line)</span>
          <textarea rows={3} value={settings.allowlist.join("\n")} onChange={e=>setAllowlist(e.target.value)} placeholder="you@example.com"/></label>
        <p className="hint">Email sender addresses alone are not proof of identity. Keep the Agent in read-only mode unless you have an authenticated mail channel.</p>
      </>}
      <div className="editor-actions"><button className="primary" disabled={busy} onClick={save}>{busy?"Saving…":"Save settings"}</button>
        <button className="outline" onClick={()=>setEditing(null)}>Close</button></div>
    </section>}
    <section className="summary">
      <div className="summary-top"><strong>Gateway activity</strong><span><i className={settings.enabled?"green-dot":"gray-dot"}/>{settings.enabled?"Active":"Stopped"}</span></div>
      <div className="metrics">
        <div><b>{stats?.running_jobs??0}</b><span>Running</span></div>
        <div><b>{stats?.pending_jobs??0}</b><span>Queued</span></div>
        <div><b>{formatLast}</b><span>Last mail</span></div>
      </div>
    </section>
    <div className="bottom">
      <label className="switch-row"><span>Enable gateway</span>
        <input type="checkbox" checked={settings.enabled} onChange={e=>patch({enabled:e.target.checked})}/></label>
      <button className="outline" onClick={()=>setAdvanced(!advanced)}><Settings2 size={14}/> Advanced settings <ChevronDown size={14}/></button>
    </div>
    {advanced&&<section className="editor">
      <Field label="Mailbox poll interval (seconds)" type="number" value={String(settings.poll_seconds)}
        onChange={v=>patch({poll_seconds:Number(v)})}/>
      <Field label="Progress email schedule (minutes, comma-separated)" value={settings.progress_minutes.join(", ")}
        onChange={v=>patch({progress_minutes:v.split(",").map(x=>Number(x.trim())).filter(x=>Number.isFinite(x)&&x>0)})}/>
      <label className="switch-row"><span>Enable workspace-write mode</span><input type="checkbox" checked={settings.workspace_write}
        onChange={e=>patch({workspace_write:e.target.checked})}/></label>
      <p className="hint">Write mode requires an EMA-Code line in every inbound email. Treat incoming mail as untrusted; do not expose high-risk actions without additional controls.</p>
      <Field label="Write-mode access code (OS Keychain)" value={secrets.access} type="password"
        onChange={v=>setSecrets({...secrets,access:v})} placeholder="Enter a strong unique code"/>
      <button className="primary full" disabled={busy} onClick={save}>Save advanced settings</button>
    </section>}
    {notice&&<p className="notice">{notice}</p>}
    <footer>No New App. Just Email.</footer>
  </main>;
}
function ConfigRow({icon,label,value,onClick}:{icon:ReactNode;label:string;value:string;onClick:()=>void}){
  return <button className="config-row" onClick={onClick}><span className="left">{icon}<span>{label}</span></span>
    <span className="right"><span>{value}</span><ChevronDown size={15}/></span></button>;
}
function Field({label,value,onChange,placeholder,type="text"}:{
  label:string;value:string;onChange:(v:string)=>void;placeholder?:string;type?:string
}){
  return <label className="field"><span>{label}</span><input type={type} value={value}
    onChange={e=>onChange(e.target.value)} placeholder={placeholder}/></label>;
}
