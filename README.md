# EMA — Email-Native Agent Gateway

**No New App. Just Email.**

EMA is a local-first agent gateway. Send an email to your configured inbox, and EMA will route the request to a local Codex CLI running in an authorized project directory. Reply to the same email thread to continue the existing Codex session. The desktop app only configures the gateway; it is not an end-user chat client.

> Early implementation for [Issue #1](https://github.com/tans/ema/issues/1). This is experimental software that can start local processes. **Do not expose it to untrusted senders or sensitive projects.** Real IMAP/SMTP and Codex integration still require on-device verification.

## Architecture

- Rust Core: mail polling, SMTP outbox, SQLite sessions/jobs/messages, routing, Codex subprocess and 10/30/60-minute status emails.
- Tauri 2 desktop app: tray-resident local process; a single compact React/TypeScript configuration window.
- SQLite SSOT: email thread association, individual jobs, status events, queued outbound mail.
- OS credential store: mail password, optional router API key and write-mode access code.
- Existing local Codex CLI: EMA never edits Codex configuration, provider credentials, Skills or MCP servers.

Inspired by [CC Switch](https://github.com/farion1231/cc-switch) (Rust/Tauri/SQLite/React/shadcn architecture). EMA is an independent implementation, not a fork.

## Prerequisites

- Rust and Cargo; Node.js 20+ and npm; [Tauri 2 platform prerequisites](https://v2.tauri.app/start/prerequisites/).
- Codex CLI installed and authenticated (\`codex --version\`).
- A dedicated IMAP/SMTP mailbox allowing TLS and an application password.
- A trusted test project directory. The initial workspace sandbox is **read-only**.

## Local development

~~~bash
npm install
npm run tauri dev
~~~

For a release build:

~~~bash
npm run tauri build
~~~

The configuration UI runs in a compact local window. Closing the window hides it to the tray. Quitting the tray item stops the gateway.

## Setup

1. In **Email connection**, fill the mailbox address, login username, IMAP host/port (default TLS 993), SMTP host/port (default TLS 465), and the app password. The password is saved in the OS credential store, not config.json.
2. In **Local Agent**, point to the installed \`codex\` command if it is not already in PATH.
3. In **Authorized projects**, add at least one project alias and an absolute path.
4. In **Email access**, add the allowed sender addresses. **A From address alone does not authenticate email**. For actual external usage, use an authenticated, trusted mailbox/channel; for development use only your own test accounts.
5. Optionally set a model and OpenAI-compatible endpoint under **AI routing**. The router selects only from the configured projects. With one project, a model is optional.
6. Save and then toggle **Enable gateway** on. It is deliberately disabled by default.

Send an email to the gateway:

~~~text
To: agent@example.com
Subject: [codex:myproject]

Please review the README and summarize the project.
~~~

With one authorized project, the \`[codex:...]\` prefix is optional. Every accepted email creates a queued Job. The reply includes the Session's email thread; replying will continue its Codex session.

### Write-mode security

Write mode is **off by default**. It adds \`--sandbox workspace-write\` to Codex and requires the first line of **every** inbound email to match a locally configured access code:

~~~text
EMA-Code: your-long-unique-access-code
Please implement the requested change.
~~~

The access code is removed before the task enters SQLite or the model. This is only a stopgap, **not** a complete authenticated mail protocol or an approval system. Do not enable write mode for arbitrary network emails or sensitive repositories. Additional action-level approvals and origin verification remain open work.

## Progress notifications

The gateway records JSONL activity from Codex and sends a status email when a running task reaches 10, 30 and 60 minutes (configurable). Afterwards it repeats every 60 minutes. It reports only observed activity and does not invent a completion percentage. A final success or failure email is queued when the CLI exits.

Incoming messages are deduplicated by RFC Message-ID. SMTP messages are queued in SQLite and retried on failure. A process interrupted during an app restart is marked \`interrupted\` rather than silently executed again.

## Configuration and data

Tauri's application data directory contains \`config.json\` and \`ema.db\`. No mailbox password or model API key is stored there. For headless or development environments without a system keychain, credential environment variable fallbacks are supported:

- \`EMA_MAIL_PASSWORD\`
- \`EMA_ROUTER_API_KEY\`
- \`EMA_MAIL_ACCESS_CODE\`

Use operating-system secret injection. Never commit these variables or an environment file containing them.

## Known limitations

- Only Codex is currently executable; Claude Code/OpenCode adapters are not implemented.
- IMAP currently uses polling rather than IDLE; polling of **unread messages** is designed for a dedicated mailbox. Do not mark incoming messages as read using another mail client.
- The gateway does not yet independently verify SPF/DKIM/DMARC, provide OAuth mailbox login, or implement action-level email approvals. Whitelisting the From address is **not** sufficient authentication.
- Rich attachments and proactive \`mailagent send\` CLI are not implemented.
- Interrupted jobs are marked as interrupted, not automatically resumed; manual inspection is required.
- SMTP is at-least-once: a crash after remote acceptance but before updating SQLite can cause duplicate delivery.
- A live round-trip through a real mailbox and local Codex is required before production use.

## 3-minute manual demonstration

1. Launch the desktop app, configure a dedicated test inbox and a read-only sample project, enable Gateway.
2. Send \`[codex:sample]\` and ask for a short README summary.
3. Observe a queued acknowledgement and final result in the original email thread.
4. Reply asking a follow-up question, then verify the same native Codex session is reused.
5. Close the settings window and verify the tray remains active; reopen settings to inspect the activity counter.

The GitHub Issue remains the source of truth for outstanding work; please avoid creating transient plan documents.
