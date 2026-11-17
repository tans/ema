# EMA Agent Hook and CLI compatibility (v0.1.9)

EMA has **two independent flows**:

- **Email → Agent:** an allowlisted sender sends a message whose subject starts with an exact, user-editable prefix. The CLI is resolved at execution time. If a binary or adapter is missing, EMA replies in the same email thread. The incoming gateway can be disabled separately from local reports.
- **Local Agent → Email:** while EMA runs with a configured mailbox, it reconciles supported hooks on startup and periodically, and sends short result/status reports from the configured mailbox to itself. No Hook switch, separate SMTP password or second email recipient. It does not require the session to have been launched by EMA.

## Supported integrations

| Prefix | Email-driven CLI execution | Automatic external-session Hook | Notes |
| --- | --- | --- | --- |
| codex | Yes | Yes, `notify` | Does not overwrite an existing non-EMA `notify` |
| cc | Yes, Claude Code | Yes, `Stop` | Non-interactive JSON; native Agent permissions |
| gemini | Yes | Yes, `AfterAgent` | Non-interactive JSON; native Agent permissions |
| opencode | Yes | Yes, `session.idle` | Plugin loaded on next session |
| pi | Yes | Yes, `agent_settled` | Global extension |
| omp | Yes | Yes, `agent_end` | User-level observer |
| kimi | No execution adapter; error receipt | Yes, `Stop` / `StopFailure` | Config format/version dependent |
| zcode | No execution adapter; error receipt | Yes, `Stop` | Preserves explicitly disabled hook policy |
| hermes | No execution adapter; error receipt | Yes, `on_session_end` | User-scoped Hermes plugin, summary is status-only |
| openclaw | No execution adapter; error receipt | Yes, `agent_end` | Native plugin, conversation access needed for last reply |
| alma | Yes; no default subject prefix | Yes, `chat.message.didReceive` | Main reply only; live config reload |
| dsh | No execution adapter; error receipt | Conditional, `turn/end` | Uses an **already-installed** `dsh-hooks` profile plugin |

Email execution adapters exist for Codex, Claude Code, Gemini, OpenCode, Pi, OMP and Alma. Alma continues native threads and can create a new thread when an Alma prefix is configured. Remaining prefixes produce a threaded adapter-unavailable error; they are **not executed through Codex**. Each adapter leaves permissions to native Agent defaults/user configuration or the original session. Headless authorization failures are returned without automatic approval.

All installed hooks are **best effort**. An Agent may require an application/profile restart to load changes, and an upstream install can have different event payloads from the documented version. CI compilation does not replace testing each installed Agent on a real workstation.

## What automatic registration changes

EMA writes only its own managed file(s) and does not overwrite a file lacking the `EMA-MANAGED-HOOK` marker. When an existing JSON/TOML/YAML config is edited, it first creates an `*.ema-before-hook.bak` backup. Existing unrelated hook entries remain present. Unparseable configs, explicit user disable states, incompatible schemas and conflicting exclusive notifier rules are skipped and logged, not silently repaired.

- **Hermes:** installs `~/.hermes/plugins/ema-report/plugin.yaml` and `__init__.py`, adds the plugin to `plugins.enabled` in `~/.hermes/config.yaml` unless explicitly disabled. The metadata-only lifecycle event `on_session_end` includes success/failure state but does **not** provide the final reply text. Hermes may need restart after a plugin change.
- **OpenClaw:** installs the local `~/.openclaw/extensions/ema-report` native plugin and adds an enabled `plugins.entries.ema-report` entry. OpenClaw's documented `agent_end` permission requires `hooks.allowConversationAccess: true`; the plugin sends the latest available user question and final assistant text, but its host-level grant can technically access conversation content. EMA does not override an explicit deny, disabled plugin, or restrictive allowlist. Review this privacy behavior before running EMA against sensitive OpenClaw sessions.
- **DSH:** if an existing `~/.dsh/profiles/<profile>/cordis.patch.yml` already contains the `dsh-hooks` plugin, EMA appends a `turn/end` rule invoking its local script, using the plugin's `DSH_HOOK_*` environment variables. **EMA never installs the third-party `dsh-hooks` package automatically**; profiles without it are skipped. Restart/reload the affected DSH profile to take effect.

Reports follow EMA's language setting and the shared [mail format v1](MAIL_FORMAT.md). Subjects use `[EMA][AGENT][state] project · title`, falling back from native session title to the current instruction. Bodies contain a summary, status, Agent/project, instruction (2,000 characters), up to two recent conversation excerpts (1,000 each), final response (12,000), next step and session tracking. Missing responses and truncation are explicit. A round ending does not certify the user's goal is achieved. Reply instructions appear only when the native session ID exists and the adapter supports resumption. SQLite dedup, SMTP retry and EMA-started Codex duplicate suppression remain in use.

Codex titles come from its local session index; recent context is read only when the native session and turn can be matched. Claude Code may read its matching local transcript for the user question. Pi and OpenCode use their native session interfaces. These lookups collect only user or assistant text excerpts, and do not send reasoning, tool output or complete transcripts. Agent sessions that cannot supply or reliably match this information are marked unavailable.

## Alma registration and lifecycle

Alma uses `~/.config/alma/hooks.json`. EMA appends one command to
`hooks["chat.message.didReceive"]` with matcher `.*`, `timeout: 3000` (milliseconds)
and `enabled: true`. The command is an absolute, shell-quoted `hooks/emit.sh alma`
path under EMA's data directory, so Alma's working directory does not matter.
The command receives `{hook,input,matcher}` JSON on stdin, including EOF without a
newline. It emits no stdout and performs no network or SMTP operations.

Only the main reply event is registered. Subagent completion is not registered.
`input.threadId` identifies the session; `input.message.id` identifies a reply for
stable deduplication, even if usage or JSON serialization changes. Result text
comes from `input.response.content` (string or text parts), with assistant
`input.message.parts` as a fallback. EMA also looks up the matching thread title,
user question and up to two preceding user/assistant messages through the reply's
own parent chain in Alma's local database. Reasoning, tool parts and usage are not
included; the report uses the excerpt limits above.
Missing thread/message IDs and other event types are discarded. When no explicit
`input.cwd` is supplied, the project is reported as `unknown`; EMA never uses the
hook process's arbitrary current directory as the project.

Alma currently emits this completion event through its usage-dependent success
path. Replies without usage, cancelled runs and failed runs are **not guaranteed
to emit a hook**. EMA cannot report an event that Alma does not deliver. File
changes reload automatically; no Alma restart or `alma hooks` CLI is needed.

The gateway reconciles registration through the existing startup/minute loop.
For explicit management without starting mail workers:

```sh
cargo build -p ema-core --release --bin ema-alma-hook
./target/release/ema-alma-hook install
./target/release/ema-alma-hook status
./target/release/ema-alma-hook uninstall
# An alternate absolute EMA data directory may be supplied as the second argument.
```

On macOS the default data directory is
`~/Library/Application Support/xin.ema.gateway`. `status` reports registration,
enabled state, executable availability and whether the collector heartbeat is
fresh. A registered hook only captures while EMA has a configured mailbox and
refreshes `hooks/active` (20-second freshness limit). Installation alone does not
start EMA or enable email. Existing disabled EMA entries remain disabled.
Uninstall removes only this data directory's Alma command, retains other hooks
and the shared shim, and writes `hooks/alma.disabled` so automatic reconciliation
does not reinstall it. Explicit `install` removes that marker. Non-empty configs
are backed up to `hooks.ema-before-hook.bak` before their first change; invalid
JSON or incompatible schemas are preserved and explicit installation returns an
error. The local Alma API can inspect the loaded configuration with
`GET http://localhost:23001/api/hooks`.

For SMTP-free verification, run `cargo test -p ema-core alma_`. The tests execute
the configured shell command from `/` with a newline-free Alma payload, using a
private disposable queue and SQLite database under this repo's `target/hook-tests`.
They run only the hook collector, never mail workers, and check that duplicate
replies yield one queued report and zero sent reports. Temporary files are removed
when each test finishes.

## Operational verification

Use a controlled, non-sensitive email account:

1. Start EMA with SMTP properly configured; check its log for Hook reconciliation warnings.
2. Start an independent CLI session and let it complete a short innocuous task. For Hermes, expect status-only; for other supported Agents, expect the result text when supplied by that Agent.
3. Check the **Local Agent reports** counters on EMA's status overview: captured / sent / pending are separate from incoming task counters.
4. Verify one message is delivered to the EMA mailbox, that its subject begins with `[EMA]`, and that no loop-created Agent tasks appear.
5. Repeat while inbound Email → Agent is disabled; reporting should still work.
6. If the Agent is missing, has its hooks disabled, requires permission approval, or has an incompatible config, verify EMA leaves its existing configuration intact.

### Security and privacy

Do **not** attach the full transcript, secrets, environment variables or file contents to a Hook email. A final reply may still accidentally include sensitive content. Any user running EMA and its automatic HookManager should understand that **local Agent results are sent to their email provider**. This Alpha has no cryptographic sender authentication on inbound email and should use controlled sender addresses and reviewed native Agent permissions. EMA does not override tool availability, sandbox/approval policy, or automatically approve an interactive authorization request.

Official references: [OpenClaw plugin hooks](https://docs.openclaw.ai/plugins/hooks), [Hermes plugin events](https://hermes-agent.nousresearch.com/docs/user-guide/features/hooks), [DSH Hooks](https://github.com/PeterBon/dsh-hooks).
