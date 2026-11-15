# EMA Agent Hook and CLI compatibility (v0.1.9)

EMA has **two independent flows**:

- **Email → Agent:** an allowlisted sender sends a message whose subject starts with an exact, user-editable prefix. The CLI is resolved at execution time. If a binary or adapter is missing, EMA replies in the same email thread. The incoming gateway can be disabled separately from local reports.
- **Local Agent → Email:** while EMA runs with a configured mailbox, it reconciles supported hooks on startup and periodically, and sends short result/status reports from the configured mailbox to itself. No Hook switch, separate SMTP password or second email recipient. It does not require the session to have been launched by EMA.

## Supported integrations

| Prefix | Email-driven CLI execution | Automatic external-session Hook | Notes |
| --- | --- | --- | --- |
| codex | Yes | Yes, `notify` | Does not overwrite an existing non-EMA `notify` |
| cc | Yes, Claude Code | Yes, `Stop` | Non-interactive `plan` mode |
| gemini | Yes | Yes, `AfterAgent` | Non-interactive sandbox/plan mode |
| opencode | Error receipt until approved runner exists | Yes, `session.idle` | Plugin loaded on next session |
| pi | Error receipt until approved runner exists | Yes, `agent_end` | Global extension |
| omp | Error receipt until approved runner exists | Yes, `agent_end` | User-level observer |
| kimi | Error receipt until approved runner exists | Yes, `Stop` / `StopFailure` | Config format/version dependent |
| zcode | Error receipt until approved runner exists | Yes, `Stop` | Preserves explicitly disabled hook policy |
| hermes | Error receipt until approved runner exists | Yes, `on_session_end` | User-scoped Hermes plugin, summary is status-only |
| openclaw | Error receipt until approved runner exists | Yes, `agent_end` | Native plugin, conversation access needed for last reply |
| dsh | Error receipt until approved runner exists | Conditional, `turn/end` | Uses an **already-installed** `dsh-hooks` profile plugin |

Only three email execution adapters exist; the other eight prefixes are still recognized and produce a threaded error email. They are **not executed through Codex**. This is intentional: no unverified write-capable headless CLI commands are used for inbound email instructions.

All installed hooks are **best effort**. An Agent may require an application/profile restart to load changes, and an upstream install can have different event payloads from the documented version. CI compilation does not replace testing each installed Agent on a real workstation.

## What automatic registration changes

EMA writes only its own managed file(s) and does not overwrite a file lacking the `EMA-MANAGED-HOOK` marker. When an existing JSON/TOML/YAML config is edited, it first creates an `*.ema-before-hook.bak` backup. Existing unrelated hook entries remain present. Unparseable configs, explicit user disable states, incompatible schemas and conflicting exclusive notifier rules are skipped and logged, not silently repaired.

- **Hermes:** installs `~/.hermes/plugins/ema-report/plugin.yaml` and `__init__.py`, adds the plugin to `plugins.enabled` in `~/.hermes/config.yaml` unless explicitly disabled. The metadata-only lifecycle event `on_session_end` includes success/failure state but does **not** provide the final reply text. Hermes may need restart after a plugin change.
- **OpenClaw:** installs the local `~/.openclaw/extensions/ema-report` native plugin and adds an enabled `plugins.entries.ema-report` entry. OpenClaw's documented `agent_end` permission requires `hooks.allowConversationAccess: true`; the plugin sends the latest available user question and final assistant text, but its host-level grant can technically access conversation content. EMA does not override an explicit deny, disabled plugin, or restrictive allowlist. Review this privacy behavior before running EMA against sensitive OpenClaw sessions.
- **DSH:** if an existing `~/.dsh/profiles/<profile>/cordis.patch.yml` already contains the `dsh-hooks` plugin, EMA appends a `turn/end` rule invoking its local script, using the plugin's `DSH_HOOK_*` environment variables. **EMA never installs the third-party `dsh-hooks` package automatically**; profiles without it are skipped. Restart/reload the affected DSH profile to take effect.

Reports follow EMA's language setting. Subjects use the available native session title, then the current question, then Agent/status as a fallback. Bodies contain the Agent, session ID, project folder, question, up to two recent conversation excerpts and final response when available. The question is capped at 4,000 characters, each context excerpt at 1,000 and the response at 10,000; omissions and truncation are indicated. Different Agent events provide different fields. The native session ID can itself be a local path. Email subjects start with `EMA Report`; SQLite dedup and SMTP retry remain in use. EMA-started Codex sessions are not reported twice.

Codex titles come from its local session index; recent context is read only when the native session and turn can be matched. Claude Code may read its matching local transcript for the user question. Pi and OpenCode use their native session interfaces. These lookups collect only user or assistant text excerpts, and do not send reasoning, tool output or complete transcripts. Agent sessions that cannot supply or reliably match this information are marked unavailable.

## Operational verification

Use a controlled, non-sensitive email account:

1. Start EMA with SMTP properly configured; check its log for Hook reconciliation warnings.
2. Start an independent CLI session and let it complete a short innocuous task. For Hermes, expect status-only; for other supported Agents, expect the result text when supplied by that Agent.
3. Check the **Local Agent reports** counters on EMA's status overview: captured / sent / pending are separate from incoming task counters.
4. Verify one message is delivered to the EMA mailbox, that its subject begins with `EMA Report`, and that no loop-created Agent tasks appear.
5. Repeat while inbound Email → Agent is disabled; reporting should still work.
6. If the Agent is missing, has its hooks disabled, requires permission approval, or has an incompatible config, verify EMA leaves its existing configuration intact.

### Security and privacy

Do **not** attach the full transcript, secrets, environment variables or file contents to a Hook email. A final reply may still accidentally include sensitive content. Any user running EMA and its automatic HookManager should understand that **local Agent results are sent to their email provider**. This Alpha has no cryptographic sender authentication on inbound email and should use controlled sender addresses and read-only local Agent tasks.

Official references: [OpenClaw plugin hooks](https://docs.openclaw.ai/plugins/hooks), [Hermes plugin events](https://hermes-agent.nousresearch.com/docs/user-guide/features/hooks), [DSH Hooks](https://github.com/PeterBon/dsh-hooks).
