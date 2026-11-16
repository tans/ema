# cc-switch（farion1231/cc-switch）Hook 与配置管理技术调研报告

> 调研对象：`~/code/cc-switch`（Tauri 桌面应用，Rust 后端约 35 万行 + React 前端）
> 调研日期：2026-07
> 目的：为 EMA 的 agent hook 注入 / 配置管理寻找可借鉴的设计。
> 性质：纯研究，未修改 cc-switch 任何代码。

---

## 0. 结论速览（TL;DR）

1. **cc-switch 本身不是一个 hook 框架，它不向任何 agent CLI 注册 hook**。它是一个"配置切换器"：切换 provider 时**只替换客户端配置文件里的关键字段**（endpoint / key / 模型 / 协议），用户自己的 plugins、hooks、MCP、注释、排版全部原样保留。README 原话（`README_ZH.md:462`）：

   > 不会。切换供应商时，CC Switch 只替换配置文件里的**关键字段**……插件、Hook、权限、MCP、你自己加的环境变量、注释和排版都原样保留，对所有供应商生效。

2. cc-switch 里"hook"出现在三种形态（详见 §1）：
   - 作为**用户拥有的配置字段**被刻意**不碰**（`hooks` 键、Codex 的 `notify`）；
   - 作为**会话日志里的执行结果**被 session manager 解析成 UI 事件（Claude 的 `stop_hook_summary`）；
   - 作为 **Rust 内部回调机制**（SQLite update hook、模式发布 hook、panic hook）。

3. 它最有价值的部分不是 hook 本身，而是**"受控地改写别人拥有的配置文件"这一整套工程**：所有权模型（floor）、幂等保序补丁（patch）、原子写入引擎（engine）、带崩溃恢复的操作事务（operation/WAL）、契约化免写（contract）、敏感键剥离。这些是 EMA 目前 `crates/core/src/hooks.rs` 可以系统性对标的对象（详见 §7）。

4. 代码库中**不存在** `hooks.json` 写入、shell notify 脚本生成、系统环境变量注入（grep 全库确认）。它最接近"脚本注入"的功能是 MCP server 命令同步（把 stdio MCP 命令写进 `~/.claude.json` / Codex `config.toml` 的 `[mcp_servers]`）和 provider 级 JS 用量查询脚本（进程内 QuickJS 沙箱执行，不是 shell）。

---

## 1. cc-switch 中 hook 是什么概念

### 1.1 三种"hook"

| 形态 | 位置 | 说明 |
| --- | --- | --- |
| ① 用户拥有的配置字段 | `~/.claude/settings.json` 的 `hooks` 键；`~/.codex/config.toml` 的 `notify` 键等 | cc-switch **明确不拥有、不写、不删**。切换 provider 时这些键字节级保留。 |
| ② 会话日志中的 hook 执行记录 | Claude Code session JSONL 的 `system` / `stop_hook_summary` 记录 | session manager 解析 `hookErrors`，非空时产出 `EventKind::Hook` 事件展示在会话查看器里。 |
| ③ Rust 内部回调 | `database/mod.rs:129`（SQLite `update_hook`）、`mode/operation.rs:554-586`（发布前 hook）、`panic_hook.rs` | 与本调研主题无关，仅为说明 "hook" 一词在代码库里的全部含义。 |

**关键证据**：

- `src-tauri/src/deeplink/codex_link.rs:6`（Codex 深链解析的模块说明）：
  > MCP、sandbox、notify 等）不收，切换本来也不写它们。

  —— Codex `config.toml` 的 `notify`（正是 EMA 注入的那个键）被 cc-switch 划入"用户/客户端拥有"范围。

- `src-tauri/src/live/project/claude.rs:3`（Claude 直连投影模块说明）：
  > 行里其余的键（旧版回填进来的插件、hooks，深链带进来的超时设置等）不投影：它们归用户和客户端，新版只从 live 里读写它们。

- `src-tauri/src/live/floor.rs` 全篇没有 `hooks` / `notify` 键：floor（关键字段清单，见 §4.1）里只有连接与鉴权相关键，证明 hook 类键**不在所有权声明内**。

### 1.2 注册与触发

- **注册**：cc-switch 不注册任何 agent hook。唯一与"hook 配置"相关的写操作是 MCP 同步（§5.1），写的是 MCP 命令而不是 hook。
- **触发**：cc-switch 不触发 hook。它只**读取** Claude 会话日志里 Stop hook 的执行摘要（§2）。
- 内部回调 hook（形态③）的注册方式（供参考）：`mode/operation.rs` 里用 `static RefCell<Option<PublishHook>>` 槽位，`on_before_publish(Some(Box::new(...)))` 注册、发布时调用；测试用它做故障注入（failpoint 风格的崩溃点模拟）。

---

## 2. Hook 的事件类型

### 2.1 cc-switch 感知到的 Claude hook 事件

全库 grep `PreToolUse|PostToolUse|SessionStart|SessionEnd|UserPromptSubmit|SubagentStop|Notification` 无任何命中（只有前端 `routeSwitchNotification` 这类同名 UI 工具函数）。cc-switch **只消费一种 hook 结果**：

```rust
// src-tauri/src/session_manager/providers/claude.rs:482-497
Some("stop_hook_summary") => {
    let errors: Vec<String> = match &record.hook_errors {
        Some(Value::Array(items)) => items.iter()
            .map(|item| match item {
                Value::String(s) => s.trim().to_string(),
                other => other.to_string(),
            })
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    };
    if !errors.is_empty() {
        self.push_event(ts, EventKind::Hook, Some(errors.join("\n")), None);
    }
}
```

即：Claude Code 会话 JSONL 里 `{"type":"system","subtype":"stop_hook_summary","hookErrors":[...]}` 记录，`hookErrors` 非空才生成事件（`model.rs:278` 注释："Claude system.stop_hook_summary（有错误时才产出）"）。

### 2.2 session manager 的 UI 事件枚举（非 hook 事件）

`src-tauri/src/session_manager/model.rs` 的 `EventKind`：

```rust
pub enum EventKind {
    Aborted,        // 用户中断 / turn_aborted
    Compaction,     // 上下文压缩（Codex compaction、Pi、Claude /compact）
    ModelChange,    // Pi model_change、Codex thread_settings_applied
    ThinkingLevel,  // Pi thinking_level_change
    Hook,           // Claude system.stop_hook_summary（有错误时才产出）
    PrLink,         // Claude pr-link
    SlashCommand,   // Claude <command-name>
    Info, Error,    // Gemini
    SubAgent,       // Codex agent_message / SubAgentActivity
    Other,
}
```

这是 cc-switch **会话查看器**的事件模型（从本地 JSONL/rollout 文件解析，`session_manager/providers/{claude,codex,gemini,...}.rs`，各 1000~3000 行解析器）。EMA 的"agent 会话状态推送"如果要做展示层，这套按 provider 分文件的解析器结构可直接参考。

### 2.3 对照：各 agent 原生 hook 事件（cc-switch 不感知，EMA 在用）

cc-switch 没有统一 hook 事件表；EMA 目前实际使用的原生事件（`ema/docs/AGENT_HOOKS.md`）：

| Agent | 配置文件 | 事件 | 注入形态 |
| --- | --- | --- | --- |
| codex | `~/.codex/config.toml` | `notify` | TOML 数组 `["shim codex","codex"]` |
| claude | `~/.claude/settings.json` | `Stop` | JSON `hooks.Stop[]` |
| gemini | `~/.gemini/settings.json` | `AfterAgent` | JSON `hooks.AfterAgent[]` |
| opencode | `~/.config/opencode/plugins/ema-report.js` | `session.idle` | 整个插件文件 |
| pi | `~/.pi/agent/extensions/ema-report.ts` | `agent_settled` | 整个扩展文件 |
| omp | `~/.omp/agent/hooks/post/*.ts` | `agent_end` | 整个观察者文件 |
| kimi | `~/.kimi-code/config.toml` | `Stop`/`StopFailure` | TOML `[[hooks]]` 数组表 |
| hermes / openclaw / alma / dsh | 各自插件/hooks.json | `on_session_end` / `agent_end` / `chat.message.didReceive` / `turn/end` | 插件或 hooks.json 条目 |

---

## 3. 配置方式与文件格式

cc-switch 管理的文件格式（对应各 agent）：

| Agent | 文件 | 格式 | 补丁器 |
| --- | --- | --- | --- |
| Claude Code | `~/.claude/settings.json` | JSON（`preserve_order`） | `live/patch/json.rs`（JsonPatch） |
| Claude Code MCP | `~/.claude.json` | JSON | `claude_mcp.rs`（只重写 `mcpServers` 键） |
| Codex | `~/.codex/config.toml` | TOML（`toml_edit`） | `live/patch/toml.rs`（TomlPatch） |
| Codex 登录 | `~/.codex/auth.json` | JSON | 整体原子写（0600） |
| Gemini CLI | `~/.gemini/.env` 等 | dotenv | `live/patch/dotenv.rs` |
| 自身 | `~/.cc-switch/`：`cc-switch.db`（SQLite，provider 行）、`live-state.json`（模式/pending 状态）、`backups/` | — | — |

**没有 hooks.json 概念**（Alma 的 `~/.config/alma/hooks.json` 是 Alma 产品自己的，cc-switch 不认识；EMA 写它是 EMA 与 Alma 之间的契约）。

### JSON 补丁语义（`live/patch/json.rs:26-37`）

```rust
pub struct JsonPatch {
    /// 先清空的关键字段。目标值里也有的键不删，留给 set 原位改值。
    pub clear: Vec<ClearScope>,        // 按谓词（floor 判定）清空一个作用域
    pub remove: Vec<KeyPath>,          // 按路径点名删除
    pub set: Vec<(KeyPath, Value)>,    // 已存在→原位改值；不存在→追加到父对象末尾
    pub seed: Vec<(KeyPath, Value)>,   // 缺失时才写入
    pub remove_if: Vec<(KeyPath, Vec<Value>)>,      // 当前值精确命中才删
    pub remove_when: Vec<(KeyPath, ValuePredicate)>,// 当前值满足谓词才删
}
```

要点：
- `serde_json` 开 `preserve_order` 保键序；删除用 `shift_remove`（`Map::remove` 在 preserve_order 下等于 `swap_remove`，会挪乱键序）；
- 重新序列化保留原文件的**缩进风格（空格/Tab）、CRLF、BOM、末尾换行**（`JsonStyle`，`json.rs:141-230`），但不保证字节级排版稳定（第一次写入会规范化转义/空白，之后字节稳定）——文档里明确承认这个取舍；
- 文件不存在或空白 → 空对象是唯一允许的"空底"；顶层不是对象 → 报错不覆盖。

### TOML 补丁语义（`live/patch/toml.rs:21-32`）

基于 `toml_edit`：注释、空行、键的写法和表的位置**原样保留**，只改补丁点名的键。`set` 原位替换时用 `keep_layout`（`toml.rs:151`）沿用旧条目的装饰（空白/行尾注释）和表位置，替换后这一行/这张表不挪动。`remove_if` 的值比较忽略两侧空白与行尾注释（`same_value`）。

---

## 4. 如何修改/管理 CLI agent 的配置文件

这是 cc-switch 的核心。一句话：**把"改别人家的文件"做成一个所有权明确、可重放、可恢复的受控写系统**。

### 4.1 所有权模型：floor（地板）/ 关键字段

`src-tauri/src/live/mod.rs:1-12`（模块总说明）：

```
//! 所有权层：CC Switch 只拥有客户端配置里的关键字段，其余字节不碰。
//! - floor：每个应用的关键字段和供应商独有字段（纯函数谓词和常量清单）；
//! - project：纯函数投影器，供应商行 → 关键字段和独有字段的目标值；
//! - patch：按格式保序改写（JSON、TOML、.env），解析不了就停；
//! - residue：兼容期残留清理（冻结的键值对清单）；
//! - engine：写锁、计划、临时文件、首写备份、重读比对。
```

关键字段回答四个问题（`floor.rs:1-12`）："请求发到哪、凭什么鉴权、哪个模型名、说哪种协议"。它们完全归供应商所有，切换时**一律清空再写目标供应商的值**；其余键归用户，不写也不删。

```rust
// live/floor.rs:60
pub const CLAUDE_FLOOR_TOP: &[&str] = &["apiKeyHelper","apiBaseUrl","primaryModel",
    "smallFastModel","apiKey","model","fallbackModel","modelOverrides","modelPicker",
    "advisorModel","awsAuthRefresh","awsCredentialExport","gcpAuthRefresh"];
// live/floor.rs:24-26
pub const CLAUDE_FLOOR_ENV_PREFIXES: &[&str] = &["ANTHROPIC_", "AWS_", "VERTEX_REGION_"];
// live/floor.rs:121
pub const CODEX_FLOOR_TOP: &[&str] = &["model_provider","openai_base_url","model",
    "review_model","model_reasoning_effort", ... "wire_api"];
// live/floor.rs:150 —— CC Switch 声明"整张表"归自己
pub const CODEX_PROVIDER_TABLE: &[&str] = &["model_providers", "custom"];
```

注意前缀匹配的**安全方向设计**（`floor.rs:15-22`）：只有"整个前缀都属于连接和鉴权"的前缀才按前缀匹配；漏掉的键会被当用户键保留，"失败方向是安全的"。`CLAUDE_CODE_USE_*` 特意不按前缀（同前缀下还有与供应商无关的功能开关），改用显式清单 `CLAUDE_PROTOCOL_SELECTORS`。

前端有同一份键表的镜像 `tests/config/claudeKeyFields.json`，由 Rust 侧测试生成并与前端预设扫描互校（`floor.rs:40-43` 注释）——**前后端所有权清单双写互测**，防止两边漂移。

### 4.2 投影：provider 行 → 补丁

```rust
// live/project/claude.rs:89-115  （直连切换的补丁构造，核心逻辑 25 行）
pub fn direct_patch(prev: Option<&ClaudeProjection>, target: &ClaudeProjection) -> JsonPatch {
    // 独有字段：先删 prev 带进来、而且值没被改过的，再写 target 的
    let outgoing = prev.into_iter().flat_map(|prev| &prev.exclusive)
        .map(|(key, value)| (env.child(key), vec![value.clone()]));
    let residue = residue::CLAUDE_RESIDUE_ENV.iter()
        .map(|(key, values)| (env.child(key), residue::residue_values(values)));
    JsonPatch {
        clear: vec![  // 关键字段：清空整个作用域（floor 谓词）
            ClearScope { parent: KeyPath::root(), is_floor: floor::claude_floor_top },
            ClearScope { parent: env,            is_floor: floor::claude_floor_env },
        ],
        set: target.set_entries(),                 // 写 target 的关键字段 + 独有字段
        remove_if: outgoing.chain(residue).collect(), // 删上一家的独有字段（值没被改过才删）
        ..Default::default()
    }
}
```

三条规则（`claude.rs:84-88` 注释）：
- **关键字段**：一律清空，再写 target 的；
- **独有字段**（供应商兼容开关、上下文窗口值，如 `CLAUDE_CODE_DISABLE_ARTIFACT`）：切入时写入，切走时**只删上一家带进来、而且值没被用户改过的**（`remove_if` 精确值匹配）；
- **残留清理**（`live/residue.rs`）：冻结旧版曾下发过的 (键,值) 对（如 `CLAUDE_CODE_MAX_CONTEXT_TOKENS=262144`），每次直连投影时精确命中即删——处理"旧版整份写入留下的有害值"这种删不干净的历史残留。

**编辑器预览与真实切换用同一个补丁**：`project_onto`（`claude.rs:118`）在内存里算出"切完长什么样"，UI 显示的 diff 和落盘的字节来自同一函数——所见即所得。

### 4.3 写引擎：plan / stage / 首写备份 / 重读比对

`src-tauri/src/live/engine.rs`（改客户端文件的**唯一底层路径**）：

```
读字节并记 hash → 解析（失败就停，什么都不写）→ 内存里改 → plan（记 pre/planned hash）
→ stage（写目标旁边的临时文件，fsync，崩溃后可靠前滚）
→ ensure_first_write_backup（每个文件首次经引擎写入前，留一份原文件字节级备份；
   以文件绝对路径的 hash 前 12 位为键，备份目录 <device>/backups/live-first-write/，
   旁边的 .source 文件记原路径）
→ rename 前最后重读一次比对 hash：没变才 rename；变了以新内容为底重算（最多 3 次）
```

其他要点：
- `LiveFile::private`：含 Key 的文件（settings.json、Codex config.toml、auth.json）按 **0600** 写（`config.rs:446 atomic_write_private`，临时文件就创建成 0600，不出现先 0644 后收紧的窗口）；
- 应用级写锁 `lock_app(app)`（`engine.rs`）：Mutex+Condvar 表，同一 app 的所有写入方（切换、编辑器、模式操作、托盘）串行，不可重入；
- 乐观锁的诚实声明（`engine.rs` 头部注释）：外部进程（Claude Code 自己）不参与这把锁，"重读比对"与 rename 之间仍有极小窗口——cc-switch 选择**用重读比对把窗口缩到最小并检测**，而不是假装没有；
- `plan().is_noop()`（pre hash == planned hash）：目标内容 == 现状就完全不动文件（不改 mtime、不备份、不写临时文件）。

### 4.4 操作事务：pending WAL + 前滚恢复

`src-tauri/src/mode/operation.rs`（1204 行）把"一次切换"当成**一个操作**来提交：

```rust
// mode/operation.rs:1-19 模块说明（摘录）
//! - 发布前的失败（解析失败、路由校验不通过、并发冲突）什么都不改；
//! - 一旦开始发布，就以 pending 为准前滚补完。写到一半失败或进程崩溃，
//!   下次操作这个应用或下次启动时按 pending 补完剩下的文件和状态（指针等）。
```

执行序列（`run()`，`operation.rs:65-206`）：
1. 先在内存算好**每个**文件（任一解析失败 → 整体不写）；
2. 备好**所有**临时文件（fsync 落盘）；
3. 写下写前意图 `Pending` 到 `~/.cc-switch/live-state.json`（每个文件记 `pre` hash、`planned` hash、临时文件路径；`published: bool`）；
4. 逐个发布（rename）；**换进第一个文件之前先置 `published=true` 并持久化**——否则"文件已换 + 客户端又改了它"时恢复阶段分不出发布是否开始过；
5. 全部发布后 `commit_target`（改指针/模式/写入记录，**必须可重复执行**），再清 pending。

恢复算法（`recover()`，注释 `operation.rs:14-19`）：按文件比对当前内容与 pre/post hash——
- 没有文件是"写后内容"且 `published=false` → 还没开始发布，丢弃 pending；
- 已开始发布 → 用临时文件前滚补完仍是写前内容的文件；**被外部改过的文件（两者都不是）以外部为准不再动**，状态照样落定（例：Codex 自己刷新了刚写的 `auth.json`）。

配套：`mode/contract.rs` 的**契约键** = `sha256(关键字段 + 独有字段 + 代理地址 + 版本)`（`contract.rs:35-48`）。代理模式下切换路由时，新路由契约与当前摘要相同 → **客户端文件不读不写**，只换代理上游；不同才在同一个操作里先改写客户端再发布路由。这是"契约不变就免写"的幂等免写优化。

切换主流程串联（`services/provider/mod.rs:7074 switch` → `switch_normal` → `switch_claude_direct`）：

```rust
// services/provider/claude_direct.rs:31-40
pub(crate) fn switch_to(db, prev, target) {
    write(db, prev, target, Some(&target.id))  // 同一个操作里写 live + 改指针
}
// 内部：AppWrite::begin(db, app()) 会先 recover 上一次未完成的 pending，
// 再 plan → stage → set_pending → 逐个发布 → commit_target（指针）→ 清 pending
```

Codex 的 `switch_codex_direct`（`provider/mod.rs:7421` 附近）把 **config.toml + auth.json + 模型目录 + 指针**放进同一个操作提交；切换后重投影 MCP（幂等维护），失败**降级为警告**而不报"切换失败"（"MCP 投影可自愈：下次切换/任一 MCP 启停都会重新投影"，`mod.rs:7258` 附近注释）——关键路径与附属路径的错误分级。

### 4.5 备份体系（多层）

- **首写备份**（engine，每文件一次，路径 hash 为键）——防引擎自身 bug；
- **完整备份/恢复**（`database/backup.rs`，SQLite 级别，含 `import_authorizer` 钩子控制导入时的写表权限）；
- **环境变量备份**（`services/env_manager.rs`：删系统环境变量前先写 `~/.cc-switch/backups/env-backup-<ts>.json`，含冲突来源与值，可一键恢复；Windows 走注册表、Unix 走 shell rc 文件行过滤）。

---

## 5. Shell 脚本 hook、环境变量注入、API key 路由

### 5.1 Shell 命令写入（最接近"脚本 hook"的功能：MCP 同步）

cc-switch 不生成 shell hook 脚本，但会把 MCP server 的 stdio 命令**注入/同步**进两个 agent 配置：

- Claude：`src-tauri/src/mcp/claude.rs:41-45` → `claude_mcp.rs:321 set_mcp_servers_map`：读 `~/.claude.json`，**只覆盖 `mcpServers` 这一个键**，其他字段不动；写入前做平台修正（Windows 上 `npx` 包成 `cmd /c npx`；检测到 WSL 路径跳过包装）；UI 辅助字段（`enabled/source/id/name/...`）写入前剔除（`claude_mcp.rs:338-349`）。
- Codex：`src-tauri/src/mcp/codex.rs:308 sync_enabled_to_codex` / `441 sync_single_server_to_codex`：同样"只动 `[mcp_servers]` 表"的投影。
- 语义：cc-switch 把"启用的 MCP 集合"视为**自己拥有的投影**（DB 行 → live 文件的幂等投影，每次切换后重投影，见 §4.4）；`should_sync_claude_mcp()`（`mcp/claude.rs:13-17`）在 Claude 未安装（目录与文件都不存在）时**跳过写、不创建任何文件/目录**——"agent 不装就不碰盘"的保守原则。

### 5.2 环境变量注入

- **不做系统级环境变量注入**。`services/env_manager.rs` 只做"检测到的环境变量冲突 → 备份 → 删除/恢复"（用户主动确认场景）。
- 所谓"env 注入"实际是**写进客户端配置文件的 `env` 对象**：
  - 直连：provider 行的 `env` 关键字段（`ANTHROPIC_BASE_URL`/`ANTHROPIC_AUTH_TOKEN`/…）投影进 `settings.json`；
  - 代理模式：`live/project/claude.rs:199` 的占位凭据
    ```rust
    /// 代理模式下写进客户端的凭据占位符。旧版只认这个字面值来识别接管态，不能改。
    pub const PROXY_TOKEN_PLACEHOLDER: &str = "PROXY_MANAGED";
    ```
    `proxy_projection`（`claude.rs:249`）把 `ANTHROPIC_BASE_URL` 指向本地代理地址、凭据写成 `PROXY_MANAGED`、模型名写成稳定别名（`claude-sonnet-5` 等，`claude.rs:206-211`，注释解释了别名要跟上代际模型的原因，#8118）。真实凭据留在 cc-switch 自己的代理里。
- **敏感键剥离**：`services/provider/mod.rs:7647 is_sensitive_config_key`——把"通用配置片段"（会合并进每个勾选供应商的共享 env）里的凭据键按模式**从严**剥掉（`_KEY`/`_TOKEN`/`_PAT`/`_PWD`/`_SECRETKEY` 等后缀 + `SECRET`/`CREDENTIAL` 等包含匹配 + 精确名单）。注释明确安全取向："故意从严——多剥一个非机密键只是小不便，漏剥一个凭据则会把密钥注入到每个供应商（不可恢复的泄漏）"。

### 5.3 API key 路由（本地代理模式）

cc-switch 的"路由模式"= **本地 HTTP 反向代理 + 客户端指向 127.0.0.1:port**：

- 客户端文件里不写真实 Key（只写 `PROXY_MANAGED` 占位），真实 Key 只在 cc-switch 进程内；
- `proxy/provider_router.rs` 的 `ProviderRouter`：按"当前路由指针 / 故障转移队列 + 熔断器（CircuitBreaker，半开探测名额用 guard 管理）"选出可用上游 provider 列表（`select_providers`，`provider_router.rs:52-80`）；
- 请求转发/协议转换在 `proxy/providers/*`（Claude/Codex/Gemini 的 API 互转、SSE 流式、usage 统计）；
- 配套：`proxy/failover_switch.rs`（切换）、`proxy/switch_lock.rs`（防止切换与在途请求打架）、`proxy/privacy_filter/`（PII/密钥脱敏）、`proxy/circuit_breaker.rs`（熔断）；
- **官方账号保护**：`provider_supports_failover`（`provider_router.rs:23-28`）——Codex 官方账号卡不参与 failover（"复用其入站 token 打到别的账号卡会跨账号边界"）；`switch` 入口禁止路由模式下切官方供应商（"可能导致账号被封禁"，`mod.rs:7185` 附近）。

对 EMA 的提示：EMA 目前是"agent → codex-hook binary → 本地服务"的**单向推送**路由；cc-switch 的契约键 + 熔断/故障转移是"双向配置面"的参考，但 EMA 若只做状态上报，不需要引入本地代理。

### 5.4 provider 级 JS 用量脚本（受限脚本执行的范本）

`src-tauri/src/usage_script.rs`（728 行）：每个 provider 可配一段 JS 脚本查余额/用量，**在进程内 QuickJS 沙箱执行**，不是 shell：

```rust
// usage_script.rs:27-33
const USAGE_SCRIPT_TIMEOUT_SECS: u64 = 5;
const USAGE_SCRIPT_MEMORY_LIMIT_BYTES: usize = 16 * 1024 * 1024;
// set_max_stack_size(256KiB) + set_interrupt_handler（每轮解释器循环检查超时，
// 超时抛不可捕获异常）—— 防 while(true) 挂死后端（有对应回归测试）
```

安全面：模板变量 `{{apiKey}}/{{baseUrl}}/{{accessToken}}` 替换；请求 URL **强制 HTTPS + 与 base_url 同源**（自定义模板模式放宽，用户自担风险）；返回结构强类型校验。这是"执行不可信来源脚本"的完整参考实现（deeplink/同步导入都可能带脚本）。

---

## 6. 架构总结

```
┌────────────────────────── 前端 (React) ──────────────────────────┐
│ 供应商卡片/编辑器（预览=后端同一补丁的内存投影）  会话查看器(EventKind) │
│ 用量面板 ← tauri 事件 "usage-log-recorded"（200ms 防抖 emit）      │
└──────────────────────────────┬───────────────────────────────────┘
                               │ Tauri commands
┌──────────────────────────────┴───────────────────────────────────┐
│                        Rust 后端                                  │
│  ProviderService::switch ──► mode::controller (直连/路由/Stack 模式)│
│     │                                                            │
│     ├─► services/provider/{claude,codex,gemini,grok}_direct.rs   │
│     │      （每个 app 的写入口；Claude Desktop/累加式 app 另路）      │
│     │                                                            │
│     ├─► live/project/<app>.rs   投影：provider 行 → 补丁（纯函数）   │
│     │       live/floor.rs        关键字段/独有字段清单（所有权声明）   │
│     │       live/residue.rs      旧版残留 (键,值) 冻结清单           │
│     │                                                            │
│     ├─► live/patch/{json,toml,dotenv}.rs  保序保样式补丁器          │
│     │                                                            │
│     ├─► live/engine.rs  plan→stage(临时文件+fsync)→首写备份→重读比对  │
│     │       config.rs   atomic_write_private(0600)/StagedWrite    │
│     │                                                            │
│     └─► mode/operation.rs  pending WAL(live-state.json)+前滚恢复   │
│             mode/state.rs  Pending/PendingFile/ModeState          │
│             mode/contract.rs 契约 sha256 → 不变则免写客户端文件      │
│                                                                    │
│  SQLite(~/.cc-switch/cc-switch.db): provider 行 + 当前指针 + 模式   │
│  代理模式: proxy/* (ProviderRouter + 熔断 + failover + 协议转换)    │
│  会话查看: session_manager/* (读各 agent 本地 JSONL/rollout)        │
│  MCP 同步: mcp/* → claude_mcp.rs / codex_config (只动 mcpServers)  │
└───────────────────────────────────────────────────────────────────┘
```

核心不变量：
1. **所有权**：每个写入的键都能指认"归供应商（floor）/归供应商独有（exclusive）/归用户（不碰）"；
2. **幂等**：投影是纯函数；`is_noop` 免写；MCP 每次切换重投影；契约不变免写；
3. **可恢复**：写前意图（pending）+ fsync 临时文件 + 前滚/丢弃判定 + 外部修改优先；
4. **可预览**：编辑器与落盘共用同一补丁函数；
5. **可回退**：首写备份（每文件一次）+ 完整备份 + env 备份；
6. **安全**：0600 写含 Key 文件；敏感键从严剥离；不可信脚本沙箱执行；官方账号禁 failover。

---

## 7. 对 EMA 的具体借鉴点

EMA 现状（`crates/core/src/hooks.rs` + `docs/AGENT_HOOKS.md`）已有的好基础：`EMA-MANAGED-HOOK` 标记、按条去重（`name=ema-report` / notify 数组成员判定）、`backup_once` 首改备份、`atomic_write`（0600+fsync+rename）、解析失败即跳过、"notify 已被占用则保留用户 notifier"。以下按优先级列出可借鉴项：

### P0 —— 正确性/健壮性

1. **rename 前重读比对（防 agent 自写覆盖）**。cc-switch `engine.rs`：发布前重读文件、与 pre hash 比对，变了就以新内容为底重算（≤3 次）。EMA 的 `install_codex` 读 `config.toml` → 注入 → `atomic_write`，中间 Codex 进程若在运行时改了 `config.toml`，EMA 会用旧底覆盖它。EMA 的补丁是**幂等追加**（重放安全），所以只需最小改造：rename 前重读、若 hash 变了就在新内容上重放注入逻辑一次（注入是 `get("notify").is_none()` 判定的天然幂等操作）。这一条收益最大、成本最小。

2. **多文件 reconcile 的写前意图（轻量 pending）**。EMA 单次 reconcile 会写多个文件（Codex config.toml + Claude settings.json + 各插件文件 + alma hooks.json）。当前"启动/分钟级对账 + 重读补偿"能自愈，但崩溃窗口内可能半套。可借鉴 `mode/operation.rs`：reconcile 前写一个 `~/.ema/hooks-pending.json`（文件清单 + 各自 pre hash + 是否已开始），完成即删；启动时若发现残留按 pre/post 判定补完或丢弃。若认为 EMA 场景风险低，至少把**"备份在写入前落盘"**（现状 `backup_once` 已满足）与**"每个文件的写入结果（成功/跳过/冲突）记日志"**补齐，让半套状态可诊断。

3. **写锁串行化同 agent 的多个写入方**。EMA 有三个写入方：gateway 启动 reconcile、分钟级 reconcile、`ema-alma-hook install` CLI（独立进程）。当前只靠 atomic rename（保证单次写不撕裂，不防 read-modify-write 竞态：两个进程同时读 → 各自注入 → 后者覆盖前者的备份判定）。借鉴 `engine.rs::lock_app`：单进程内 per-agent `Mutex`；跨进程用文件锁（`flock` 一个 `~/.ema/lock-<agent>`）。cc-switch 的锁不可重入 + Condvar 的写法可直接抄（约 40 行）。

### P1 —— 配置管理工程化

4. **把 11 个 `install_*` 收敛为数据驱动的 AgentSpec 表**。cc-switch 的 `AppType::spec()`（`app_config.rs:528-543`）是编译期强制完整表："不要加 `_ =>` 分支：新增应用必须在这里写全每一项"。EMA 可定义：

   ```rust
   struct AgentHookSpec {
       agent: &'static str,          // codex / claude / ...
       discover: fn(&Path) -> Option<HookTarget>,  // 目录存在性探测（含 .kimi-code/.kimi 回退这类逻辑）
       kind: HookKind,               // TomlArrayNotify | JsonHooksEntry | PluginFile | HooksJsonCommand | ...
       event: &'static str,          // "notify" / "Stop" / "agent_settled" / ...
       ownership: Ownership,         // EntryPredicate（按条目判定归属）| WholeFile（带 marker）
       payload: fn(&ShimPath) -> Value,
       removable: bool,             // 卸载语义
   }
   ```
   当前每个 `install_*` 手写"探测 → 解析 → 去重 → 备份 → 写"，逻辑重复且漂移风险高（例如 `install_kimi` 与 `install_codex` 的 TOML 去重判据不同但意图相同）。

5. **显式所有权声明（entry ownership）替代隐式 marker**。cc-switch 用**键路径 + 谓词**声明所有权（`remove_if`/`remove_when`/`ClearScope`），EMA 用"整文件 marker + 条目名去重"。两者都可用，但建议 EMA 把"这个条目是 EMA 的"判定统一成显式谓词（如 Codex：`notify[0..2] == [shim_path, "codex"]`；Alma hooks.json：`command` 以 `hooks/emit.sh alma` 结尾），并且**卸载（uninstall）与安装共用同一谓词**——EMA 的 `status`/`uninstall` CLI 已经需要这套判定，集中在一处可以避免 install 说"是我的"、uninstall 找不到。

6. **字节级幂等 no-op + 风格保留**。cc-switch `plan().is_noop()`（hash 相等 → 完全不碰文件）。EMA 现状：JSON 用 `to_vec_pretty` 与 `old` 字符串比较——`serde_json` pretty 固定 2 空格缩进，用户文件若用 4 空格/Tab/CRLF，每次 reconcile 都会"内容语义相同但字节不同"→ 触发备份 + 重写（mtime 抖动、备份覆盖、编辑器告警）。借鉴 `JsonStyle`（`json.rs:141-230`）：探测原文件缩进/CRLF/BOM 并在序列化时沿用；TOML 侧 `toml_edit` 已保序保注释，只需保持现状即可。

7. **注册契约键（免写 + 漂移检测）**。借鉴 `mode/contract.rs`：把"当前应注册的完整状态"（agent 清单、shim 绝对路径、各 agent 事件、payload）序列化成规范 JSON 取 sha256 存为 `ema-hooks-contract`；reconcile 时先算契约，与上次相同直接 no-op（连文件都不读），UI/status 命令用它回答"注册是否漂移"。shim 路径变化（数据目录迁移）时契约自然变化 → 触发全量重注册，行为正确且可观测。

### P2 —— 可观测性与扩展

8. **前端事件推送模板（防抖 + 非阻塞）**。`usage_events.rs`（81 行）是 EMA "把 agent 会话状态推给前端"的直接模板：`OnceLock<AppHandle>`（写路径不持有句柄）+ `AtomicBool` 200ms 防抖合并 + `emit` 失败仅 warn 不传播。EMA 的 codex-hook → 前端通道若有轮询或高频 emit，直接套用此模式；事件命名（`usage-log-recorded` → 如 `agent-session-updated`）与前端 `useTauriEvent` 桥（`useUsageEventBridge.ts`）同理可抄。

9. **会话文件解析作为 hook 推送的兜底（pull 补 push）**。cc-switch `session_manager/providers/claude.rs`（2950 行）/`codex_items.rs` 等证明：本地 JSONL/rollout 的会话时间线是**可以完整重建**的，且 `stop_hook_summary` 这类记录连 hook 错误都能还原。EMA 的 AGENT_HOOKS.md 已明确某些 hook 事件**不保证发射**（Alma 失败路径不发 hook；Kimi 格式依赖版本）。可加一条兜底路径：hook payload 已带 `session_id`（多数是本地路径），前端/网关对"超时长无事件"的会话按 `session_id` 增量解析会话文件补齐状态（只读、幂等）。push（实时）+ pull（兜底）双通道，cc-switch 的解析器分层（RawRecord → 事件/块投影，每个 provider 一个 parser 文件）是现成结构参考。

10. **敏感信息处理策略**。若 EMA 将来引入"共享配置/模板"（例如多个 mailbox 共享 agent 配置片段），直接抄 `is_sensitive_config_key`（`mod.rs:7647`）的"从严模式匹配 + 注释解释每个后缀"。EMA 现状"agent 进程拿不到 SMTP 凭据"（hooks.rs 头部注释）是正确的最小面设计，保持不变。

11. **不可信脚本的受限执行**（仅当需要"用户自定义报告脚本"时）。`usage_script.rs` 的 QuickJS 沙箱（5s 超时 + 16MiB 内存 + 256KiB 栈 + interrupt handler + HTTPS/同源校验 + 返回值类型校验）是完整范本，含"无限循环脚本必须被中断"的回归测试。EMA 的 `emit.sh` 是受控模板生成（`__ACTIVE__`/`__SPOOL__` 占位替换 + shell_quote），与 cc-switch 的 MCP 命令注入类似，保持"只注入自己生成的绝对路径命令"即可，不建议开放用户自定义命令。

### 明确不需要借鉴的

- **本地代理 / API key 路由 / 熔断 failover**（§5.3）：EMA 不做 LLM 流量代理，无对应场景。
- **floor 关键字段清单**（§4.1）：EMA 不切 provider，不需要"endpoint/key/模型"所有权清单；EMA 的对应物是"hook 条目所有权"（§7.4/7.5）。
- **整份替换式备份策略**：cc-switch 首写备份 + 完整备份是给它"大面积改写"兜底；EMA 只追加条目，`backup_once` 已足够，最多把备份名带时间戳（现状固定 `*.ema-before-hook.bak`，用户手动改过后再 reconcile 不会留第二份——可接受，但值得在 status 里显示"备份时间早于最近一次修改"这类漂移提示）。

---

## 附：关键文件索引（cc-switch）

| 主题 | 路径 |
| --- | --- |
| 所有权层总说明 | `src-tauri/src/live/mod.rs` |
| 关键字段/独有字段清单 | `src-tauri/src/live/floor.rs` |
| 残留清理冻结清单 | `src-tauri/src/live/residue.rs` |
| Claude 投影/补丁/代理契约 | `src-tauri/src/live/project/claude.rs` |
| Codex 投影 | `src-tauri/src/live/project/codex.rs` |
| JSON 保序补丁器 | `src-tauri/src/live/patch/json.rs` |
| TOML 保序补丁器 | `src-tauri/src/live/patch/toml.rs` |
| dotenv 补丁器 | `src-tauri/src/live/patch/dotenv.rs` |
| 写引擎（plan/stage/备份/锁） | `src-tauri/src/live/engine.rs` |
| 操作事务 + WAL 恢复 | `src-tauri/src/mode/operation.rs` |
| pending/模式状态结构 | `src-tauri/src/mode/state.rs` |
| 契约键（免写判定） | `src-tauri/src/mode/contract.rs` |
| 切换主流程 | `src-tauri/src/services/provider/mod.rs`（`switch` @7074） |
| Claude 直连写入口 | `src-tauri/src/services/provider/claude_direct.rs` |
| Codex 直连写入口 | `src-tauri/src/services/provider/codex_direct.rs` |
| 原子写/0600/临时文件 | `src-tauri/src/config.rs`（`atomic_write_private` @446） |
| MCP → Claude 注入 | `src-tauri/src/mcp/claude.rs`、`src-tauri/src/claude_mcp.rs` |
| MCP → Codex 注入 | `src-tauri/src/mcp/codex.rs` |
| hook 执行结果解析 | `src-tauri/src/session_manager/providers/claude.rs:482` |
| UI 事件枚举 | `src-tauri/src/session_manager/model.rs` |
| 前端防抖事件推送 | `src-tauri/src/usage_events.rs` |
| 受限 JS 脚本执行 | `src-tauri/src/usage_script.rs` |
| 代理路由/熔断/failover | `src-tauri/src/proxy/provider_router.rs` |
| 敏感键剥离 | `src-tauri/src/services/provider/mod.rs:7647` |
| 环境变量备份/删除 | `src-tauri/src/services/env_manager.rs` |
| App 规格表（多 agent） | `src-tauri/src/app_config.rs:498-543` |
| 前端关键字段镜像 | `tests/config/claudeKeyFields.json` |
