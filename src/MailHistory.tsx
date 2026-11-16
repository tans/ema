import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { RefreshCw, Search } from "react-feather";
import "daisyui/theme/light.css";
import "daisyui/components/button.css";
import "daisyui/components/input.css";
import "./mail-history.css";

type MailRecord = {
  id: string; message_id: string; direction: "in" | "out";
  sender: string; mailbox: string; recipient: string; cc: string; references: string;
  original_date: string; subject: string; body: string;
  status: string; detail: string; archive_note: string;
  created_at: number | null; sent_at: number | null; attempts: number | null;
};

const copy = {
  "zh-CN": {
    title: "邮件记录", description: "查看 EMA 轮询到的新邮件和发出的邮件。记录保存在本机。",
    search: "搜索主题、正文、邮箱或邮件 ID", refresh: "刷新", loading: "正在加载邮件记录…",
    empty: "还没有邮件记录", noMatches: "没有找到匹配的邮件", emptyHelp: "启用收信后，新轮询到的邮件会保存在这里。",
    select: "选择一封邮件查看详情", in: "收件", out: "发件", from: "发件人", to: "收件人", mailbox: "收件邮箱", cc: "抄送", references: "关联邮件 ID", originalDate: "邮件日期",
    recorded: "记录时间", sentAt: "SMTP 接受时间", attempts: "发送尝试", messageId: "邮件 ID",
    content: "邮件正文", lastError: "最近一次发送错误", noBody: "没有可显示的正文", unknown: "未知", noSubject: "（无主题）",
    previous: "上一页", next: "下一页", page: "第", pageEnd: "页", failed: "加载失败，请刷新重试。",
    coverage: "仅记录开始轮询后的新邮件；不保存附件。SMTP 接受不代表投递成功。",
    statuses: {
      processing: "处理中", retry: "处理失败，等待重试", automated: "自动或自身邮件", unauthorized: "未进入任务：发件人未授权",
      empty: "未进入任务：无纯文本正文", access_denied: "未进入任务：访问码无效", owner_mismatch: "未进入任务：会话发件人不符",
      ordinary: "普通邮件", unsupported: "无法执行：Agent 不支持", not_installed: "无法执行：Agent 未安装",
      unavailable_runner: "无法执行：Agent 不可用", awaiting_directory: "等待确认目录", duplicate: "重复邮件",
      queued_in: "任务已排队", queued_out: "等待发送", sent: "SMTP 已接受", failed: "发送失败",
      sending: "正在发送", unknown: "发送状态未知", legacy: "旧记录：信息不完整", malformed: "邮件无法解析",
      too_large_for_task: "未进入任务：邮件超过处理大小", unavailable: "邮件已不可获取"
    },
    notes: {
      html: "此邮件仅有 HTML 正文，下方显示源文本。", truncated: "正文超过 20 万字符，已截断。",
      html_truncated: "此邮件仅有 HTML 正文，源文本超过 20 万字符，已截断。",
      too_large: "邮件超过 16 MiB，仅保存邮件头信息。", unreadable: "邮件正文无法解析。",
      unavailable: "服务器没有返回邮件内容。", legacy: "升级前只记录了邮件 ID；正文及主题无法从此记录还原。"
    }
  },
  en: {
    title: "Mail history", description: "Review new mail polled and sent by EMA. Records stay on this device.",
    search: "Search subject, body, address or message ID", refresh: "Refresh", loading: "Loading mail history…",
    empty: "No mail records yet", noMatches: "No matching mail", emptyHelp: "Newly polled mail appears here after receiving is enabled.",
    select: "Select a message to see its details", in: "Received", out: "Sent", from: "From", to: "To", mailbox: "Mailbox", cc: "Cc", references: "Related message IDs", originalDate: "Message date",
    recorded: "Recorded", sentAt: "Accepted by SMTP", attempts: "Send attempts", messageId: "Message ID",
    content: "Message body", lastError: "Most recent send error", noBody: "No body available", unknown: "Unknown", noSubject: "(No subject)",
    previous: "Previous", next: "Next", page: "Page", pageEnd: "", failed: "Could not load history. Refresh to retry.",
    coverage: "Only new mail polled after recording begins is included; attachments are not saved. SMTP acceptance does not guarantee delivery.",
    statuses: {
      processing: "Processing", retry: "Processing failed; retrying", automated: "Automated or own mail", unauthorized: "Not a task: sender not allowed",
      empty: "Not a task: no plain-text body", access_denied: "Not a task: invalid access code", owner_mismatch: "Not a task: session owner mismatch",
      ordinary: "Ordinary mail", unsupported: "Cannot run: Agent unsupported", not_installed: "Cannot run: Agent not installed",
      unavailable_runner: "Cannot run: Agent unavailable", awaiting_directory: "Waiting for directory", duplicate: "Duplicate mail",
      queued_in: "Task queued", queued_out: "Waiting to send", sent: "Accepted by SMTP", failed: "Send failed",
      sending: "Sending", unknown: "Send status unknown", legacy: "Older record: incomplete", malformed: "Unable to parse",
      too_large_for_task: "Not a task: message too large", unavailable: "Message no longer available"
    },
    notes: {
      html: "This message only has HTML; its source is shown below.", truncated: "The body exceeded 200,000 characters and was truncated.",
      html_truncated: "This HTML source exceeded 200,000 characters and was truncated.",
      too_large: "This message exceeded 16 MiB; only headers were recorded.", unreadable: "The body could not be parsed.",
      unavailable: "The server did not return the message body.", legacy: "Only the message ID was recorded before this update; its subject and body cannot be recovered here."
    }
  }
};

export default function MailHistory({ language }: { language: "zh-CN" | "en" }) {
  const t = copy[language];
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(0);
  const [version, setVersion] = useState(0);
  const [rows, setRows] = useState<MailRecord[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

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
        .catch(err => { if (current) setError(String(err)); })
        .finally(() => { if (current) setLoading(false); });
    }, 200);
    return () => { current = false; window.clearTimeout(timer); };
  }, [search, page, version]);

  const selectedMail = rows.slice(0, 50).find(item => item.id === selected);
  const date = (value: number | null) => value ? new Date(value * 1000).toLocaleString(language) : t.unknown;
  const status = (mail: MailRecord) => {
    const key = mail.status === "queued" ? `queued_${mail.direction}` : mail.status;
    return t.statuses[key as keyof typeof t.statuses] ?? mail.status;
  };
  const note = (key: string) => t.notes[key as keyof typeof t.notes] ?? "";

  return <section className="history-page">
    <div className="history-heading"><div><h1>{t.title}</h1><p>{t.description}</p></div>
      <button type="button" className="btn btn-sm" onClick={() => setVersion(value => value + 1)}><RefreshCw size={15}/>{t.refresh}</button>
    </div>
    <p className="history-coverage">{t.coverage}</p>
    <label className="history-search"><Search size={16}/><input className="input input-sm" type="search" aria-label={t.search}
      placeholder={t.search} value={search} onChange={event => { setSearch(event.target.value); setPage(0); }}/></label>
    {error && <p className="history-error" role="alert">{t.failed} {error}</p>}
    {loading && <p className="history-message" role="status">{t.loading}</p>}
    {!loading && !error && rows.length === 0 && <div className="history-message"><strong>{search ? t.noMatches : t.empty}</strong>
      {!search && <p>{t.emptyHelp}</p>}</div>}
    {!loading && !error && rows.length > 0 && <>
      <div className="history-layout">
        <div className="history-list" aria-label={t.title}>
          {rows.slice(0, 50).map(mail => <button type="button" key={mail.id}
            className={"history-row " + (mail.id === selected ? "selected" : "")}
            aria-pressed={mail.id === selected} onClick={() => setSelected(mail.id)}>
            <span className="history-row-top"><span>{mail.direction === "in" ? t.in : t.out}</span><time>{date(mail.created_at)}</time></span>
            <strong>{mail.subject || t.noSubject}</strong>
            <span className="history-address">{mail.direction === "in" ? mail.sender || t.unknown : mail.recipient || t.unknown}</span>
            <span className="history-status">{status(mail)}</span>
          </button>)}
        </div>
        <div className="history-detail" aria-live="polite">
          {selectedMail ? <>
            <div className="history-detail-top"><span>{selectedMail.direction === "in" ? t.in : t.out}</span><span>{status(selectedMail)}</span></div>
            <h2>{selectedMail.subject || t.noSubject}</h2>
            <dl>
              <div><dt>{t.from}</dt><dd>{selectedMail.sender || t.unknown}</dd></div>
              <div><dt>{t.to}</dt><dd>{selectedMail.recipient || t.unknown}</dd></div>
              {selectedMail.mailbox && <div><dt>{t.mailbox}</dt><dd>{selectedMail.mailbox}</dd></div>}
              {selectedMail.cc && <div><dt>{t.cc}</dt><dd>{selectedMail.cc}</dd></div>}
              {selectedMail.original_date && <div><dt>{t.originalDate}</dt><dd>{selectedMail.original_date}</dd></div>}
              <div><dt>{t.recorded}</dt><dd>{date(selectedMail.created_at)}</dd></div>
              {selectedMail.sent_at && <div><dt>{t.sentAt}</dt><dd>{date(selectedMail.sent_at)}</dd></div>}
              {selectedMail.direction === "out" && <div><dt>{t.attempts}</dt><dd>{selectedMail.attempts ?? t.unknown}</dd></div>}
              {selectedMail.message_id && <div><dt>{t.messageId}</dt><dd>{selectedMail.message_id}</dd></div>}
              {selectedMail.references && <div><dt>{t.references}</dt><dd>{selectedMail.references}</dd></div>}
            </dl>
            {selectedMail.detail && <p className="history-error"><strong>{t.lastError}: </strong>{selectedMail.detail}</p>}
            {note(selectedMail.archive_note) && <p className="history-note">{note(selectedMail.archive_note)}</p>}
            <h3>{t.content}</h3><pre>{selectedMail.body || t.noBody}</pre>
          </> : <p className="history-message">{t.select}</p>}
        </div>
      </div>
      <div className="history-pagination"><button type="button" className="btn btn-sm" disabled={page === 0} onClick={() => setPage(page - 1)}>{t.previous}</button>
        <span>{t.page} {page + 1} {t.pageEnd}</span>
        <button type="button" className="btn btn-sm" disabled={rows.length <= 50} onClick={() => setPage(page + 1)}>{t.next}</button></div>
    </>}
  </section>;
}
