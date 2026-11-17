import { notify } from "./Toast";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { ArrowLeft } from "react-feather";
import "daisyui/theme/light.css";
import "daisyui/components/button.css";
import "daisyui/components/input.css";
import "daisyui/components/badge.css";
import "./mail-history.css";
import AgentReply from "./AgentReply";
import ReactMarkdown from "react-markdown";

type MailRecord = {
  id: string; message_id: string; direction: "in" | "out";
  sender: string; mailbox: string; recipient: string; cc: string; references: string;
  original_date: string; subject: string; body: string;
  kind: string; status: string; detail: string; archive_note: string;
  created_at: number | null; sent_at: number | null; attempts: number | null;
};

const copy = {
  "zh-CN": {
    title: "邮件", back: "返回列表", metadata: "邮件信息", description: "查看 EMA 轮询到的新邮件和发出的邮件。记录保存在本机。",
    search: "搜索主题、正文、邮箱或邮件 ID", refresh: "刷新", loading: "正在加载邮件记录…",
    empty: "还没有邮件记录", noMatches: "没有找到匹配的邮件", filteredEmpty: "当前过滤条件下没有邮件", filterHint: "打开工具栏的过滤按钮可调整显示的邮件类型。", emptyHelp: "启用收信后，新轮询到的邮件会保存在这里。",
    select: "选择一封邮件查看详情", in: "收件", out: "发件", reply: "回复", from: "发件人", to: "收件人", mailbox: "收件邮箱", cc: "抄送", references: "关联邮件 ID", originalDate: "邮件日期",
    recorded: "记录时间", sentAt: "SMTP 接受时间", attempts: "发送尝试", messageId: "邮件 ID",
    content: "邮件正文", lastError: "最近一次发送错误", noBody: "没有可显示的正文", unknown: "未知", noSubject: "（无主题）",
    previous: "上一页", next: "下一页", page: "第", pageEnd: "页", failed: "加载失败，请刷新重试。",
    kinds: {
      echo: "回环", unrelated: "无关", new_task: "新任务", resume_task: "继续任务", invalid: "已拒绝",
      new_task_ack: "新任务回执", resume_ack: "继续回执", progress: "进度", result: "结果", error: "错误", hook: "Hook", test: "测试"
    }, legacyKind: "历史邮件",
    notes: {
      local_reply: "从 EMA 内提交的会话指令；回执和结果通过邮件返回。",
      html: "此邮件仅有 HTML 正文，下方显示源文本。", truncated: "正文超过 20 万字符，已截断。",
      html_truncated: "此邮件仅有 HTML 正文，源文本超过 20 万字符，已截断。",
      too_large: "邮件超过 16 MiB，仅保存邮件头信息。", unreadable: "邮件正文无法解析。",
      unavailable: "服务器没有返回邮件内容。", legacy: "升级前只记录了邮件 ID；正文及主题无法从此记录还原。"
    }
  },
  en: {
    title: "Mail", back: "Back to list", metadata: "Message information", description: "Review new mail polled and sent by EMA. Records stay on this device.",
    search: "Search subject, body, address or message ID", refresh: "Refresh", loading: "Loading mail history…",
    empty: "No mail records yet", noMatches: "No matching mail", filteredEmpty: "No mail under the current filters", filterHint: "Use the filter button in the toolbar to adjust which mail types are shown.", emptyHelp: "Newly polled mail appears here after receiving is enabled.",
    select: "Select a message to see its details", in: "Received", out: "Sent", reply: "Reply", from: "From", to: "To", mailbox: "Mailbox", cc: "Cc", references: "Related message IDs", originalDate: "Message date",
    recorded: "Recorded", sentAt: "Accepted by SMTP", attempts: "Send attempts", messageId: "Message ID",
    content: "Message body", lastError: "Most recent send error", noBody: "No body available", unknown: "Unknown", noSubject: "(No subject)",
    previous: "Previous", next: "Next", page: "Page", pageEnd: "", failed: "Could not load history. Refresh to retry.",
    kinds: {
      echo: "Echo", unrelated: "Unrelated", new_task: "New task", resume_task: "Resume", invalid: "Rejected",
      new_task_ack: "Task ack", resume_ack: "Resume ack", progress: "Progress", result: "Result", error: "Error", hook: "Hook", test: "Test"
    }, legacyKind: "Legacy",
    notes: {
      local_reply: "Conversation instruction submitted in EMA. Acknowledgment and results arrive by email.",
      html: "This message only has HTML; its source is shown below.", truncated: "The body exceeded 200,000 characters and was truncated.",
      html_truncated: "This HTML source exceeded 200,000 characters and was truncated.",
      too_large: "This message exceeded 16 MiB; only headers were recorded.", unreadable: "The body could not be parsed.",
      unavailable: "The server did not return the message body.", legacy: "Only the message ID was recorded before this update; its subject and body cannot be recovered here."
    }
  }
};

export default function MailHistory({ language, search, version, hiddenKinds = {} }:
  { language: "zh-CN" | "en"; search: string; version: number; hiddenKinds?: Record<string, boolean> }) {
  const t = copy[language];
  const [page, setPage] = useState(0);
  const [replyVersion, setReplyVersion] = useState(0);
  const [rows, setRows] = useState<MailRecord[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

  useEffect(() => { setPage(0); }, [search]);

  useEffect(() => {
    let current = true;
    setLoading(true);
    const timer = window.setTimeout(() => {
      invoke<MailRecord[]>("get_mail_history", { search, offset: page * 50 })
        .then(items => {
          if (!current) return;
          setRows(items);
          setSelected(previous => items.slice(0, 50).some(item => item.id === previous) ? previous : null);
          setError("");
        })
        .catch(err => { if (current) { setError(String(err)); notify(`${t.failed} ${String(err)}`, "error"); } })
        .finally(() => { if (current) setLoading(false); });
    }, 200);
    return () => { current = false; window.clearTimeout(timer); };
  }, [search, page, version, replyVersion]);

  const filteredRows = rows.filter(mail => !hiddenKinds[mail.kind]);
  const displayBody = (body: string) => body.replace(/^EMA-Message:[^\r\n]*(?:\r?\n)+\s*/i, "");
  const selectedMail = filteredRows.slice(0, 50).find(item => item.id === selected);
  const selectedBody = selectedMail ? displayBody(selectedMail.body) : "";
  const date = (value: number | null) => value ? new Date(value * 1000).toLocaleString(language) : t.unknown;
  const shortDate = (value: number | null) => value ? new Date(value * 1000).toLocaleString(language, {
    month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", hour12: false
  }) : t.unknown;
  const badge = (mail: MailRecord) => {
    const label = t.kinds[mail.kind as keyof typeof t.kinds] ?? (mail.kind || t.legacyKind);
    const tone = ["error", "invalid"].includes(mail.kind) ? "error"
      : ["new_task", "resume_task"].includes(mail.kind) ? "task"
      : ["result", "hook"].includes(mail.kind) ? "result"
      : ["new_task_ack", "resume_ack", "progress"].includes(mail.kind) ? "progress" : "neutral";
    return <span className={`badge badge-sm mail-kind mail-kind-${tone}`} title={mail.kind || t.legacyKind}>{label}</span>;
  };
  const note = (key: string) => t.notes[key as keyof typeof t.notes] ?? "";
  // HTML-only messages stay as raw source; everything else renders as Markdown.
  const isHtmlBody = (mail: MailRecord, body: string) =>
    mail.archive_note === "html" || mail.archive_note === "html_truncated"
    || /<[a-z][^>]*>/i.test(body);

  return <section className="history-page" aria-label={t.title}>
    {loading && rows.length === 0 && <p className="history-message" role="status">{t.loading}</p>}
    {!loading && !error && filteredRows.length === 0 && <div className="history-message"><strong>{rows.length > 0 ? t.filteredEmpty : (search ? t.noMatches : t.empty)}</strong>
      {rows.length > 0 ? <p>{t.filterHint}</p> : (!search && <p>{t.emptyHelp}</p>)}</div>}
    {!error && filteredRows.length > 0 && <>
      <div className={"history-layout " + (selectedMail ? "reading" : "")}>
        <div className="history-list" aria-label={t.title}>
          {filteredRows.slice(0, 50).map(mail => <button type="button" key={mail.id}
            className={"history-row " + (mail.id === selected ? "selected" : "")}
            aria-pressed={mail.id === selected} onClick={() => setSelected(mail.id)}>
            <span className="history-row-top"><strong>{mail.subject || t.noSubject}</strong>{badge(mail)}<time title={date(mail.created_at)}>{shortDate(mail.created_at)}</time></span>
            <span className="history-row-bottom"><span className="history-address">{mail.sender || t.unknown}</span></span>
          </button>)}
        </div>
        {selectedMail && <article className="history-detail" aria-live="polite">
            <button type="button" className="btn btn-ghost detail-back" onClick={() => setSelected(null)}><ArrowLeft size={16}/>{t.back}</button>
            <div className="history-detail-top">{badge(selectedMail)}
              <time>{selectedMail.original_date || date(selectedMail.created_at)}</time></div>
            <h2>{selectedMail.subject || t.noSubject}</h2>
            <AgentReply recordId={selectedMail.id} language={language} version={version + replyVersion} onSubmitted={() => setReplyVersion(value => value + 1)}/>
            <div className="message-meta">
              <span>{t.from}: {selectedMail.sender || t.unknown}</span>
              <span>{t.to}: {selectedMail.recipient || t.unknown}</span>
              {selectedMail.cc && <span>{t.cc}: {selectedMail.cc}</span>}
              {selectedMail.mailbox && <span>{t.mailbox}: {selectedMail.mailbox}</span>}
            </div>
            {selectedMail.direction === "out" && selectedMail.detail && <p className="history-error"><strong>{t.lastError}: </strong>{selectedMail.detail}</p>}
            {note(selectedMail.archive_note) && <p className="history-note">{note(selectedMail.archive_note)}</p>}
            {isHtmlBody(selectedMail, selectedMail.body) || !selectedMail.body
              ? <pre>{selectedMail.body || t.noBody}</pre>
              : <div className="history-markdown"><ReactMarkdown>{selectedBody}</ReactMarkdown></div>}
        </article>}
      </div>
      <div className="history-pagination"><button type="button" className="btn btn-ghost btn-sm" disabled={page === 0} onClick={() => setPage(page - 1)}>{t.previous}</button>
        <span>{t.page} {page + 1} {t.pageEnd}</span>
        <button type="button" className="btn btn-ghost btn-sm" disabled={filteredRows.length <= 50} onClick={() => setPage(page + 1)}>{t.next}</button></div>
    </>}
  </section>;
}
