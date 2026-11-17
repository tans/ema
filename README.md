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

For new setups the guide can enable EMA after verified IMAP login, a saved allowlist, a successful SMTP test and a detected Codex runner. Existing configured users are not forced through onboarding after upgrading. The onboarding completion flag is stored in `config.json`.

EMA no longer reads the OS credential store. After upgrading, existing users need to re-enter the mailbox authorization code in **Settings → Email connection**; previously stored keychain entries are left untouched. Until then, email sending and receiving will not work. Credentials are stored as **plain text** in `~/.ema/secrets.json` (the `.ema` directory in the user profile on Windows). On macOS and Linux, newly saved files are readable and writable only by the current user (`0600`). Other processes running as that user can read the file.

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
| Password | QQ 邮箱授权码，明文存于用户目录的 `.ema/secrets.json` |

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
- **Local credential file**: mailbox authorization code is stored in `.ema/secrets.json` in the user directory; on macOS and Linux, newly saved files use owner-only permissions (`0600`).

Optional environment variables for headless/dev usage: `EMA_MAIL_PASSWORD`, `EMA_ROUTER_API_KEY` (legacy), and `EMA_MAIL_ACCESS_CODE` (legacy write-mode). Never commit them or pass them to Agent subprocesses.

## Known limitations

- Only Codex currently has a runnable adapter. Claude Code/OpenCode detection does not mean they can execute requests yet.
- IMAP polling handles unread messages, not IDLE. Use a dedicated mailbox.
- Sender addresses are **not proof of authentic email identity**. Authentication and action-level approvals remain open work.
- Beginning a new session at the user's home directory is a convenience, **not a safe isolation boundary**. Read-only does not necessarily prevent the Agent from reading sensitive data. Use a trusted machine and never enable workspace-write for memory mode.
- IMAP connection test only checks login, not SMTP send; real QQ Mail and other providers require an end-to-end mailbox test.
- Agent native memory depends on what the installed CLI actually supports. No new EMA memory engine is implemented.
- Official Tauri signed updates require a configured signing key and a published latest.json. Startup checks are silent; download/install require confirmation in Settings → Updates. Platform signing remains ad-hoc/unsigned. See [update deployment and limitations](docs/RELEASING.md).

## Manual acceptance scenario

1. Configure an isolated QQ Mail account with an authorization code; verify IMAP test.
2. Detect local Codex, whitelist your separate sender mailbox, enable and save.
3. Send a read-only request without a project alias; Codex should use its native context or ask for clarification.
4. Receive the result, reply to the same email and verify the original Codex session resumes.
5. Close the window; verify the tray remains active and preferences persist.

### ZCode completion reporting (v0.1.8)

ZCode's documented user-level `Stop` process Hook is now part of EMA's automatic HookManager. If `~/.zcode/cli/` exists, EMA merges an idempotent `Stop` handler into `~/.zcode/cli/config.json` with the local `emit.sh zcode` callback. The resulting JSON event includes the native session ID, workspace path, and `last_assistant_message`; EMA sends a short report to the configured EMA mailbox using its existing private spool and SMTP queue. It does not require a second Hook UI, SMTP configuration, or plugin installation.

EMA preserves all existing ZCode event arrays and writes a `.ema-before-hook.bak` backup before modifying a non-empty config. If the user explicitly configured `hooks.enabled: false`, it is **not** overridden: automatically enabling that would also enable unrelated third-party hooks the user intentionally disabled. Hooks are snapshotted per session, so already-running ZCode sessions may need restarting. This integration is based on the [ZCode Hook specification](https://zcode.z.ai/en/docs/hooks) and still needs live testing on an installed ZCode build.

## Agent compatibility and local completion reporting (v0.1.9)

Replies to local completion reports are matched to the original Agent and session.
An acknowledgement lists the session, Agent, task and last report; once SMTP accepts
the acknowledgement, EMA starts the queued continuation and sends its result in the same
thread. Mail is still detected by IMAP polling (30 seconds by default), and failed
SMTP sends are retried. Duplicate messages do not start the task twice. The sender
must be allowlisted, including when replying from the EMA mailbox itself.

Continuation adapters are available for Codex, Claude Code, Gemini, OpenCode, Pi,
OMP and Alma. They require a valid original session and an available local runner;
old reports without a usable working directory may ask for clarification. Alma
continuations require Alma to be running and, under EMA's read-only setting, run
without tools. Other Agents receive a clear unsupported notice rather than being
resumed as a different Agent. Actual CLI compatibility varies by installed version.

The EMA status overview now separates **Email → Agent** tasks from **Local Agent → Email** completion reports. Eleven text prefixes are recognized, and unsupported email execution commands receive an informative reply instead of being silently ignored.

Native local Hook reporting currently covers Codex, Claude Code, Gemini, OpenCode, Pi, OMP, Kimi, ZCode, Hermes, OpenClaw and Alma (main replies). DSH reporting is supported when the DSH profile has already installed the third-party `dsh-hooks` plugin. No additional Hook settings screen is required. Hook installation preserves explicit user disables and existing third-party rules.

New sessions started by email subject prefixes currently support Codex, Claude Code and Gemini CLI. A Hook adapter is **not** proof that the agent supports safe non-interactive email execution. See the [compatibility table and privacy/verification instructions](docs/AGENT_HOOKS.md) for important limitations.

## Agent routing update (v0.1.7)

All eleven default subject prefixes (`codex`, `cc`, `opencode`, `pi`, `omp`, `zcode`, `kimi`, `dsh`, `hermes`, `openclaw`, `gemini`) are always eligible for exact leading-token routing. The UI no longer offers per-Agent enable checkboxes or manual executable scans. Prefix text remains editable. Existing `agent_routes.enabled` fields are retained for backward compatibility but ignored by the router. An allowed sender's message using an unavailable or unsupported runner receives a **threaded, deduplicated error email** rather than being ignored. Ordinary mail still does not trigger an Agent.

Email execution adapters now exist for **Codex**, **Claude Code (`cc`)** and **Gemini CLI (`gemini`)**. They use their installed global commands, with fallback resolution for common macOS GUI PATH locations. Codex retains its sandbox settings; Claude uses noninteractive JSON output in `plan` permission mode, Gemini uses JSON output with both `--sandbox` and `--approval-mode plan`. These two new adapters deliberately allow **read-only operations only**: execution errors, missing credentials, permission failures, and unavailable local sandbox dependencies are emailed back as task failures. Session IDs are retained when CLI JSON includes one. **All other routing prefixes are recognized but return an explicit adapter-unavailable error**; they are not secretly executed by Codex.

Local Agent completion Hooks are now registered for **seven** agents: Codex, Claude Code, Gemini CLI, OpenCode, Pi, **Kimi Code** (`Stop` / `StopFailure` in its TOML config), and **OMP** (`agent_end` user-scope extension). Newly observed agents will sync on the next HookManager pass, without UI switches. The remaining Hook and CLI integrations are still to be verified; do not assume all eleven have native execution support.

**Safety:** sender whitelist alone is not cryptographic authorization. Native Agent CLI `plan` flags and sandbox availability differ by installed version; use trusted test mailboxes and check local permissions before sending real instructions. The default report email is still the EMA mailbox itself. Hook reports are short and may include sensitive text, so evaluate local workflow privacy before deploying.

## Automatic local Agent completion reports (v0.1.6)

**No separate Hook switches or SMTP settings.** As soon as EMA starts, its HookManager automatically installs/reconciles completion callbacks for locally configured **Codex, Claude Code, Gemini CLI, OpenCode and Pi**. Sessions started outside EMA (including Terminal and IDE sessions) send a short completion email **from and to the EMA mailbox address**. Email-triggered Codex sessions retain their existing single-threaded EMA reply, without duplicate Hook reports. This works even if inbound email-task polling is disabled.

Hook implementations:

| Agent | Native completion event | Global location |
| --- | --- | --- |
| Codex | `notify = ["/.../emit.sh", "codex"]` / `agent-turn-complete` | `~/.codex/config.toml` |
| Claude Code | `Stop` command hook | `~/.claude/settings.json` |
| Gemini CLI | `AfterAgent` command hook | `~/.gemini/settings.json` |
| OpenCode | `session.idle` global plugin | `~/.config/opencode/plugins/ema-report.js` |
| Pi | `agent_settled` extension (fires once, after retries/compaction settle) | `~/.pi/agent/extensions/ema-report.ts` |

EMA registers hooks only for existing Agent configuration directories, and rechecks every minute, so an Agent installed later can be detected without adding a UI toggle. Existing hooks are retained; unparsable settings and unsupported or conflicting Codex `notify` commands are **not overwritten**. Before first modification, EMA writes a `*.ema-before-hook.bak` copy of any existing settings file. EMA only rewrites plugin files carrying its `EMA-MANAGED-HOOK` marker. Agent processes already running may need a restart/reload before they read new hooks.

Hook callbacks only spool up to 64 KiB of JSON into the private local EMA app data folder; they do not have mailbox credentials or perform network requests. A short-lived heartbeat makes them inactive when EMA is stopped. A five-second collector processes the files, deduplicates events in SQLite and queues mail through EMA's existing SMTP retry worker. The report follows EMA's language setting and includes the available session title, project, native session ID, current question, recent conversation context and final response. The question is limited to 4,000 characters, recent context to two excerpts of 1,000 characters each, and the response to 10,000 characters. Missing information is marked rather than guessed; some Agent events provide less. Reports use the language-specific subject prefix `EMA 报告` or `EMA Report`, so they do not begin with an Agent task prefix.

**Privacy:** Starting EMA automatically activates local session reporting to your configured mailbox. Session titles, user questions, recent conversation excerpts and Agent responses can contain sensitive text. EMA may read the relevant native local session data to find them, and sends the available excerpts to the configured mailbox automatically. It does not email full transcripts or environment variables, and no email is sent until an EMA mailbox address is configured. On Windows the installation of native hooks is not yet implemented; the current patch-release policy prioritizes macOS Apple Silicon. Remaining Agent runners and Hook adapters are tracked separately.

## Release platform policy

EMA uses version-driven installation packages:

- **Patch X.Y.Z (Z > 0)**, such as v0.1.4: **macOS M-series / Apple Silicon only** (ARM64 DMG).
- **Minor or major X.Y.0**, such as v0.2.0 or v1.0.0: **all desktop platforms** (macOS Apple Silicon and Intel, Windows x64, Linux x64).

This speeds up frequent patch releases. Intel Mac, Windows and Linux receive updated installers at the next minor or major release. Normal frontend/Rust CI remains enabled. See [release rules](docs/RELEASING.md).

## Releases

[Public Alpha releases](https://github.com/tans/ema/releases) · [中文安装指南](docs/INSTALL.zh-CN.md) · [Release 构建说明](docs/RELEASING.md)

macOS binaries are ad-hoc signed, not Apple notarized. Windows binaries lack a paid publisher signature. Installation may require OS-specific trust prompts. Build success is not production-security acceptance.

## Daily mailbox routing and working directories (v0.1.2)

Use your normal email account. New tasks must come from an approved sender and start with an exact plain-text prefix followed by a space. For example: `codex Review mfggo project`. Available default prefix names are `codex`, `cc`, `opencode`, `pi`, `omp`, `zcode`, `kimi`, `dsh`, `hermes`, `openclaw`, `gemini`. These are editable per Agent in the desktop configuration panel. New email-started sessions support Codex, Claude Code and Gemini; other prefixes return an explicit error. Replies to an EMA session resume by In-Reply-To / References, without a prefix.

EMA uses an IMAP UID cursor rather than unseen message flags. On first connection it starts with *future* messages (old inbox messages are not executed); regular messages are not marked read by EMA. If a session's work directory matches a known local project (prior session paths, configured legacy paths, or common developer folders like `~/code` and `~/projects`) it is pinned as `cwd` in SQLite. When a project directory cannot be identified unambiguously EMA sends a clarification email and does not run the Agent. Non-project tasks use isolated directories under EMA's application data `sessions/`. Paths in email or native Agent memory do not grant write permission, and new memory-first sessions remain read-only.

**Security:** the sender allowlist verifies only the parsed From address, **not** the sender's cryptographic identity. This Alpha is unsafe for untrusted email and sensitive local data. Keep workspace-write off until a separate authenticated approval mechanism is available.

## Local mail history

The desktop **Mail history / 邮件记录** view searches locally recorded senders, recipients, subjects, bodies and message IDs. While inbound polling is enabled, EMA records each new message it fetches from the configured INBOX, including ordinary, automated, rejected and malformed mail. Ordinary mail still does not trigger an Agent, and polling does not mark messages as read. Messages that arrive while polling is paused can be recorded when it resumes if they are still in INBOX. The first connection and an IMAP UIDVALIDITY reset start from future messages; EMA does not import or execute older mail or capture messages sent by other email clients.

Task replies, Hook reports and SMTP connection tests are recorded in the same local SQLite database. New outbound records include their sender, queue time, SMTP result, attempt count and most recent error; individual attempts are retained for troubleshooting. SMTP acceptance does not prove delivery. A connection interruption can leave delivery uncertain and retrying can result in duplicates. Records made by older versions contain only the fields those versions actually saved; unknown values are not reconstructed.

For incoming messages, EMA stores up to 200,000 characters of decoded text, or HTML source when no plain text is available. For messages larger than 16 MiB it stores only available headers, and it does not save attachments. The existing limits for executing a task remain 256 KiB per message and 20,000 characters. Lines containing `EMA-Code:` are redacted before the incoming body is stored. Other message contents can still be sensitive: records remain on the device in the app data directory's `ema.db` (on macOS, `~/Library/Application Support/xin.ema.gateway/ema.db`), with owner-only database permissions on Unix.
