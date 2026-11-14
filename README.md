# EMA — Email-Native Super Agent

**No New App. Just Email.**

EMA is a local Rust/Tauri email gateway for an installed **Codex CLI**. Send a message whose subject begins with an enabled Agent prefix from an allowed mailbox; EMA creates a native agent session and sends a result by email. Reply to the same thread to continue that agent session. The desktop UI has a compact status overview, an optional first-run setup wizard, and settings—not a chat client.

> Experimental Alpha. EMA can run local programs and currently does not independently authenticate email origin or provide comprehensive action approvals. Use a dedicated test mailbox and keep the Agent read-only. Do not connect sensitive workstations or repositories to untrusted email.

## Development

Requirements: Rust + Cargo, Node.js 20+, [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/), installed and authenticated `codex` CLI.

```bash
npm install
npm run tauri dev
```

```bash
npm run tauri build
```

The app stays resident in the tray when its configuration window closes.

## First-time setup and status dashboard

On a fresh installation, EMA displays a three-step **optional** onboarding guide:

1. **Connect email** — QQ Mail authorization code or another IMAP/SMTP mailbox; the connection check verifies IMAP login without changing message read status.
2. **Sender allowlist** — add trusted sender addresses, one per line.
3. **Send test email** — send a real SMTP connectivity message to a saved allowlist address. SMTP acceptance does not guarantee delivery, nor does it verify inbound Agent processing.

Every step and the whole guide can be skipped. Skipping setup never automatically enables the gateway. Once onboarding is completed, the **home screen shows only gateway enabled state and session/job statistics** (total sessions, running, queued, completed, failed and last activity). Use the top-right **Settings** button to manage mailbox credentials, sender allowlist, per-Agent subject prefixes, enable/disable, language, and advanced options. Settings also provides **Open setup guide**.

For new setups the guide can enable EMA after verified IMAP login, a saved allowlist, a successful SMTP test and a detected Codex runner. Existing configured users are not forced through onboarding after upgrading. The onboarding completion flag is stored in `config.json`; credentials continue to live in the OS credential store.

### QQ 邮箱授权码

使用 [QQ 邮箱网页端](https://mail.qq.com/) → 设置 → 账号与安全 → 安全设置，开启 IMAP/SMTP 服务并生成**授权码**。第三方客户端要输入**授权码，不是 QQ 登录密码**。

EMA 选择 **QQ 邮箱** 时自动配置：

| Setting | Value |
|---|---|
| IMAP server | `imap.qq.com` |
| IMAP port (TLS) | `993` |
| SMTP server | `smtp.qq.com` |
| SMTP port (TLS) | `465` |
| Username | 完整邮箱地址，例如 `agent@qq.com` |
| Password | QQ 邮箱授权码，存入系统 Keychain |

QQ 官方说明：[如何生成授权码](https://help.mail.qq.com/detail/106/985)。

## Native Agent memory instead of Project Directories

**Project Directories are no longer a mandatory configuration or UI item.** On creation of a new email thread, EMA checks whether the Codex CLI is installed, then starts an agent-native session from the local user's home directory in **read-only mode**. The instruction asks the installed CLI to use its **own available memory, instructions (such as AGENTS.md) and context** to determine the relevant project. EMA does **not** implement, synchronize, or fabricate a separate memory service.

If the correct project cannot be determined, the Agent should ask the sender to clarify. A memory reference to a path is **never permission** to write to that path.

On subsequent replies, EMA resumes the original Codex Session ID rather than selecting a new project.

Existing `projects` entries in old `config.json` remain readable for backwards compatibility, including explicit `[codex:alias]` subject routing. However, new sessions do not require any project alias. Legacy workspace-write mode still requires an explicitly authorized project and a per-mail access code; new memory-first sessions cannot use write mode.

## Message example

```text
From: owner@example.com
To: agent@qq.com
Subject: codex Review the project

Review the repository we discussed earlier and summarize its current state.
```

The installed CLI determines whether sufficient context exists to respond; if not, it should ask for more details. Do not assume that the Agent has memory it was never given. After an accepted message, EMA queues an acknowledgement and final email; a reply continues the native session.

## Implementation

- **Tauri 2 + React**: one compact language-aware configuration window, credential input, IMAP connection check and local Agent detection.
- **Rust Core**: mailbox IMAP polling and SMTP, sender allowlist, Codex subprocess using JSONL events, native session resume, progress notifications and persistent outbox.
- **SQLite**: sessions, jobs, messages, events, pending outbound emails.
- **OS Keychain**: mail authorization code; never in settings files or logs.

Optional environment variables for headless/dev usage: `EMA_MAIL_PASSWORD`, `EMA_ROUTER_API_KEY` (legacy), and `EMA_MAIL_ACCESS_CODE` (legacy write-mode). Never commit them or pass them to Agent subprocesses.

## Known limitations

- Only Codex currently has a runnable adapter. Claude Code/OpenCode detection does not mean they can execute requests yet.
- IMAP polling handles unread messages, not IDLE. Use a dedicated mailbox.
- Sender addresses are **not proof of authentic email identity**. Authentication and action-level approvals remain open work.
- Beginning a new session at the user's home directory is a convenience, **not a safe isolation boundary**. Read-only does not necessarily prevent the Agent from reading sensitive data. Use a trusted machine and never enable workspace-write for memory mode.
- IMAP connection test only checks login, not SMTP send; real QQ Mail and other providers require an end-to-end mailbox test.
- Agent native memory depends on what the installed CLI actually supports. No new EMA memory engine is implemented.
- This version does not enable Tauri auto-updates or paid platform signing.

## Manual acceptance scenario

1. Configure an isolated QQ Mail account with an authorization code; verify IMAP test.
2. Detect local Codex, whitelist your separate sender mailbox, enable and save.
3. Send a read-only request without a project alias; Codex should use its native context or ask for clarification.
4. Receive the result, reply to the same email and verify the original Codex session resumes.
5. Close the window; verify the tray remains active and preferences persist.

## Releases

[Public Alpha releases](https://github.com/tans/ema/releases) · [中文安装指南](docs/INSTALL.zh-CN.md) · [Release 构建说明](docs/RELEASING.md)

macOS binaries are ad-hoc signed, not Apple notarized. Windows binaries lack a paid publisher signature. Installation may require OS-specific trust prompts. Build success is not production-security acceptance.

## Daily mailbox routing and working directories (v0.1.2)

Use your normal email account. New tasks must come from an approved sender and start with an exact plain-text prefix followed by a space. For example: `codex Review mfggo project`. Available default prefix names are `codex`, `cc`, `opencode`, `pi`, `omp`, `zcode`, `kimi`, `dsh`, `hermes`, `openclaw`, `gemini`. These are editable per Agent in the desktop configuration panel. Only Codex currently has an execution adapter; the others remain disabled even if their binaries are detected. Replies to an EMA session resume by In-Reply-To / References, without a prefix.

EMA uses an IMAP UID cursor rather than unseen message flags. On first connection it starts with *future* messages (old inbox messages are not executed); regular messages are not marked read by EMA. If a session's work directory matches a known local project (prior session paths, configured legacy paths, or common developer folders like `~/code` and `~/projects`) it is pinned as `cwd` in SQLite. When a project directory cannot be identified unambiguously EMA sends a clarification email and does not run the Agent. Non-project tasks use isolated directories under EMA's application data `sessions/`. Paths in email or native Agent memory do not grant write permission, and new memory-first sessions remain read-only.

**Security:** the sender allowlist verifies only the parsed From address, **not** the sender's cryptographic identity. This Alpha is unsafe for untrusted email and sensitive local data. Keep workspace-write off until a separate authenticated approval mechanism is available.
