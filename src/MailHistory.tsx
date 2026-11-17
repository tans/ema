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
    select: "选择一封邮件查看详情", in: "收件", out: "发件", from: "发件人", to: "收件人", mailbox: "收件邮箱", cc: "抄送", references: "关联邮件 ID", originalDate: "邮件日期",
    recorded: "记录时间", sentAt: "SMTP 接受时间", attempts: "发送尝试", messageId: "邮件 ID",
    content: "邮件正文", lastError: "最近一次发送错误", noBody: "没有可显示的正文", unknown: "未知", noSubject: "（无主题）",
    previous: "上一页", next: "下一页", page: "第", pageEnd: "页", failed: "加载失败，请刷新重试。",
    kinds: {
      echo: "回环", unrelated: "无关", new_task: "新任务", resume_task: "继续任务", invalid: "已拒绝",
      new_task_ack: "新任务回执", resume_ack: "继续回执", progress: "进度", result: "结果", error: "错误", hook: "Hook", test: "测试"
    }, legacyKind: "历史邮件",
    statuses: {
      echo: "回环已过滤", missing_session: "无法续接：缺少会话", processing: "处理中", retry: "处理失败，等待重试", automated: "自动或自身邮件", unauthorized: "未进入任务：发件人未授权",
      empty: "未进入任务：无纯文本正文", access_denied: "未进入任务：访问码无效", owner_mismatch: "未进入任务：会话发件人不符",
      ordinary: "普通邮件", unsupported: "无法执行：Agent 不支持", not_installed: "无法执行：Agent 未安装",
      unavailable_runner: "无法执行：Agent 不可用", awaiting_directory: "等待确认目录", duplicate: "重复邮件",
      queued_in: "任务已排队", queued_out: "等待发送", sent: "SMTP 已接受", failed: "发送失败",
      sending: "正在发送", unknown: "发送状态未知", legacy: "旧记录：信息不完整", malformed: "邮件无法解析",
      too_large_for_task: "未进入任务：邮件超过处理大小", unavailable: "邮件已不可获取"
    },
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
    select: "Select a message to see its details", in: "Received", out: "Sent", from: "From", to: "To", mailbox: "Mailbox", cc: "Cc", references: "Related message IDs", originalDate: "Message date",
    recorded: "Recorded", sentAt: "Accepted by SMTP", attempts: "Send attempts", messageId: "Message ID",
    content: "Message body", lastError: "Most recent send error", noBody: "No body available", unknown: "Unknown", noSubject: "(No subject)",
    previous: "Previous", next: "Next", page: "Page", pageEnd: "", failed: "Could not load history. Refresh to retry.",
    kinds: {
      echo: "Echo", unrelated: "Unrelated", new_task: "New task", resume_task: "Resume", invalid: "Rejected",
      new_task_ack: "Task ack", resume_ack: "Resume ack", progress: "Progress", result: "Result", error: "Error", hook: "Hook", test: "Test"
    }, legacyKind: "Legacy",
    statuses: {
      echo: "Echo filtered", missing_session: "Cannot resume: no session", processing: "Processing", retry: "Processing failed; retrying", automated: "Automated or own mail", unauthorized: "Not a task: sender not allowed",
      empty: "Not a task: no plain-text body", access_denied: "Not a task: invalid access code", owner_mismatch: "Not a task: session owner mismatch",
      ordinary: "Ordinary mail", unsupported: "Cannot run: Agent unsupported", not_installed: "Cannot run: Agent not installed",
      unavailable_runner: "Cannot run: Agent unavailable", awaiting_directory: "Waiting for directory", duplicate: "Duplicate mail",
      queued_in: "Task queued", queued_out: "Waiting to send", sent: "Accepted by SMTP", failed: "Send failed",
      sending: "Sending", unknown: "Send status unknown", legacy: "Older record: incomplete", malformed: "Unable to parse",
      too_large_for_task: "Not a task: message too large", unavailable: "Message no longer available"
    },
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
  const selectedMail = filteredRows.slice(0, 50).find(item => item.id === selected);
  const date = (value: number | null) => value ? new Date(value * 1000).toLocaleString(language) : t.unknown;
  const status = (mail: MailRecord) => {
    const key = mail.status === "queued" ? `queued_${mail.direction}` : mail.status;
    return t.statuses[key as keyof typeof t.statuses] ?? mail.status;
  };
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
            <span className="history-row-bottom"><span className="history-address">{mail.sender || t.unknown}</span>
              <span className="history-status" title={status(mail)}>{status(mail)}</span></span>
          </button>)}
        </div>
        {selectedMail && <article className="history-detail" aria-live="polite">
            <button type="button" className="btn btn-ghost detail-back" onClick={() => setSelected(null)}><ArrowLeft size={16}/>{t.back}</button>
            <div className="history-detail-top">{badge(selectedMail)}<span>{selectedMail.direction === "in" ? t.in : t.out}</span><span>{status(selectedMail)}</span></div>
            <h2>{selectedMail.subject || t.noSubject}</h2>
            <AgentReply recordId={selectedMail.id} language={language} version={version + replyVersion} onSubmitted={() => setReplyVersion(value => value + 1)}/>
            <details className="message-metadata"><summary>{t.metadata}</summary><dl>
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
            </dl></details>
            {selectedMail.direction === "out" && selectedMail.detail && <p className="history-error"><strong>{t.lastError}: </strong>{selectedMail.detail}</p>}
            {note(selectedMail.archive_note) && <p className="history-note">{note(selectedMail.archive_note)}</p>}
            <pre>{selectedMail.body || t.noBody}</pre>
        </article>}
      </div>
      <div className="history-pagination"><button type="button" className="btn btn-ghost btn-sm" disabled={page === 0} onClick={() => setPage(page - 1)}>{t.previous}</button>
        <span>{t.page} {page + 1} {t.pageEnd}</span>
        <button type="button" className="btn btn-ghost btn-sm" disabled={filteredRows.length <= 50} onClick={() => setPage(page + 1)}>{t.next}</button></div>
    </>}
  </section>;
}
