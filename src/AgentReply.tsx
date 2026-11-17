import { notify } from "./Toast";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { CornerUpLeft, Folder, Send, Terminal } from "react-feather";
import "daisyui/components/textarea.css";

type Context = { agent: string; cwd: string; native_session: string; session_id: string | null; blocked: string | null };
type Draft = { body: string; requestId: string };
const copy = {
  "zh-CN": {
    title: "继续 Agent 会话", directory: "运行目录", session: "Agent 会话", ema: "EMA 会话", unknownDirectory: "未提供目录",
    reply: "回复此会话", instruction: "本次指令", placeholder: "补充要求，让 Agent 从当前会话继续…", cancel: "收起",
    send: "提交并继续", sending: "正在提交…", accepted: "已提交到原会话，回执和结果将通过邮件返回。", hint: "直接续接原 Agent 会话，回执和结果通过邮件返回。",
    failed: "提交失败，内容已保留。", contextFailed: "无法读取会话信息", retry: "重试", limit: "最多 12,000 字",
    blocked: { paused: "请先在设置中开启任务执行。", owner_not_allowed: "会话所属邮箱不在允许列表中。", unsupported: "此 Agent 暂不支持续接。", not_installed: "本机未找到此 Agent。", directory_unavailable: "原运行目录已不可用。", session_unavailable: "原会话文件已不可用。" }
  },
  en: {
    title: "Continue Agent conversation", directory: "Working directory", session: "Agent session", ema: "EMA session", unknownDirectory: "No directory provided",
    reply: "Reply to conversation", instruction: "Your instruction", placeholder: "Add instructions to continue this conversation…", cancel: "Collapse",
    send: "Submit and continue", sending: "Submitting…", accepted: "Submitted to the original conversation. Acknowledgment and results will arrive by email.", hint: "Continue this Agent conversation. Acknowledgment and results arrive by email.",
    failed: "Submission failed. Your draft is preserved.", contextFailed: "Could not load conversation", retry: "Retry", limit: "Up to 12,000 characters",
    blocked: { paused: "Enable task execution in settings first.", owner_not_allowed: "The conversation owner is not in the allowlist.", unsupported: "This Agent does not support resuming yet.", not_installed: "This Agent is not installed.", directory_unavailable: "The original working directory is unavailable.", session_unavailable: "The original session file is unavailable." }
  }
};

export default function AgentReply({ recordId, language, version, onSubmitted }: { recordId: string; language: "zh-CN" | "en"; version: number; onSubmitted: () => void }) {
  const t = copy[language];
  const [context, setContext] = useState<Context | null>(null);
  const [contextId, setContextId] = useState("");
  const [loadError, setLoadError] = useState("");
  const [reload, setReload] = useState(0);
  const [open, setOpen] = useState(false);
  const [drafts, setDrafts] = useState<Record<string, Draft>>({});
  const [busyId, setBusyId] = useState<string | null>(null);
  const busy = useRef(false);
  const activeId = useRef(recordId);
  activeId.current = recordId;
  const textarea = useRef<HTMLTextAreaElement>(null);
  const draft = drafts[recordId] ?? { body: "", requestId: "" };
  useEffect(() => { setOpen(false); setContext(null); setLoadError(""); }, [recordId]);
  useEffect(() => {
    let current = true;
    invoke<Context | null>("get_mail_reply_context", { recordId }).then(value => {
      if (current) { setContext(value); setContextId(recordId); setLoadError(""); }
    }).catch(error => { if (current) { setLoadError(String(error)); notify(`${t.contextFailed}: ${String(error)}`, "error"); } });
    return () => { current = false; };
  }, [recordId, version, reload]);
  useEffect(() => { if (open) textarea.current?.focus(); }, [open, recordId]);
  const submit = async () => {
    if (busy.current || !context || context.blocked || !draft.body.trim() || Array.from(draft.body).length > 12000) return;
    const targetId = recordId;
    const requestId = draft.requestId || crypto.randomUUID();
    setDrafts(previous => ({ ...previous, [targetId]: { ...draft, requestId } }));
    busy.current = true; setBusyId(targetId);
    try {
      await invoke("reply_to_mail", { recordId: targetId, requestId, body: draft.body });
      setDrafts(previous => ({ ...previous, [targetId]: { body: "", requestId: "" } }));
      notify(t.accepted);
      if (activeId.current === targetId) setOpen(false);
      onSubmitted();
    } catch (error) {
      notify(`${t.failed} ${String(error)}`, "error");
    } finally { busy.current = false; setBusyId(null); }
  };
  if (loadError) return <div className="reply-load-error"><button className="btn btn-ghost btn-xs" onClick={() => setReload(value => value + 1)}>{t.retry}</button></div>;
  if (!context || contextId !== recordId) return null;
  const blocked = context.blocked ? t.blocked[context.blocked as keyof typeof t.blocked] ?? context.blocked : "";
  const submitting = busyId === recordId;
  return <section className="agent-reply" aria-label={t.title}>
    <div className="reply-heading"><span className="reply-agent"><Terminal size={15}/>{context.agent}</span><strong>{t.title}</strong>
      {!open && <button type="button" className="btn btn-ghost btn-sm" disabled={!!blocked || busyId !== null} onClick={() => setOpen(true)}><CornerUpLeft size={14}/>{t.reply}</button>}</div>
    <dl className="reply-context">
      <div><dt><Folder size={13}/>{t.directory}</dt><dd className="reply-directory">{context.cwd || t.unknownDirectory}</dd></div>
    </dl>
    {blocked && <p className="reply-hint" role="status">{blocked}</p>}
    {open && <form className="reply-compose" onSubmit={event => { event.preventDefault(); void submit(); }}>
      <label htmlFor="agent-instruction">{t.instruction}</label>
      <textarea ref={textarea} id="agent-instruction" className="textarea textarea-ghost" rows={4} value={draft.body} placeholder={t.placeholder} disabled={submitting}
        onChange={event => { const body = event.target.value; setDrafts(previous => ({ ...previous, [recordId]: { body, requestId: "" } })); }}
        onKeyDown={event => { if ((event.metaKey || event.ctrlKey) && event.key === "Enter" && !event.nativeEvent.isComposing) { event.preventDefault(); void submit(); } }}/>
      <div className="reply-compose-footer"><span className="reply-hint">{t.limit} · ⌘ / Ctrl + Enter</span><div>
        <button type="button" className="btn btn-ghost btn-sm" disabled={submitting} onClick={() => setOpen(false)}>{t.cancel}</button>
        <button type="submit" className="btn btn-primary btn-sm" disabled={busyId !== null || !!blocked || !draft.body.trim() || Array.from(draft.body).length > 12000}><Send size={13}/>{submitting ? t.sending : t.send}</button>
      </div></div>
      <p className="reply-hint">{t.hint}</p>
    </form>}
  </section>;
}
