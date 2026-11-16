# EMA — Mail Classification Core Logic

EMA 对每一封入站邮件做显式定性（`kind`），决策链顺序如下。任何分支都不得静默落入"未分类"，未分类一律视为 `unrelated` 并记录原因。

## 1. 入站定性（`mail_inbox.kind`）

| kind | 含义 | 判定规则 |
|---|---|---|
| `echo` | EMA 自己写的外发邮件的回环（QQ 等会把它投回自家 INBOX） | 命中第 2 节的"EMA 邮件特征"，直接丢弃，不创建任务、不回复 |
| `unrelated` | 无关邮件 | 无 agent 前缀，且引用链未命中任何已知会话/hook 报告 |
| `new_task` | 新任务 | 主题以合法的 `agent 前缀 + 空格` 开头，且引用链未命中会话；发件人在 allowlist |
| `resume_task` | 继续已有会话 | `In-Reply-To`/`References` 中任一 Message-ID 命中 `messages` 表或 `message_aliases` 映射表 → 复用原 session |
| `invalid` | 被拒绝 | 发件人不在 allowlist / 空正文 / 会话 owner 不匹配 / agent 不可用等，按原错误分类记录，不执行 |

优先级（严格从上到下，先到先得）：
1. `echo` — EMA 自身特征命中（防止自触发，最高优先级）
2. 引用链命中（`messages` / `message_aliases`）→ `resume_task`
3. 主题前缀命中 → `new_task`
4. 其余 → `unrelated`

## 2. EMA 自身邮件特征（识别 echo / 自己写的邮件）

EMA 外发邮件带三重复合特征，命中任一个即可判定为 EMA 自己写的：

1. **头部**：`X-EMA-Kind: <kind>` 自定义头（出站时写入）
2. **正文特征行**：正文以 `EMA-` 开头的机器可读标记行（出站时写入，兜底头被网关剥掉的情况）
3. **Message-ID 前缀**：`<ema-` / `<ema-hook-` / `<ema-error-` / `<ema-test-`（仅作辅助，QQ 可能改写，不可单独依赖）

收到带这些特征的邮件：记录 `kind=echo`，打回环标记，不进入任务处理管线。

## 3. Message-ID 改写映射（关键）

QQ/腾讯邮箱会把外发邮件的 Message-ID 改写成 `<tencent_xxx@qq.com>`，导致：

- 原 `is_outbound()`（按原始 ema ID 查 outbox）失效 → 自己的邮件被当新入站处理 → **邮件回环风暴**（见 09:07 实测，1→2→4→8 指数爆炸）
- 真实用户回复携带的引用是改写后的 tencent ID，对不上库里存的 ema ID → 回复静默变 `unrelated`（"回复没反应"的根因）

机制：`message_aliases` 表维护 `改写后 Message-ID → 原始 outbox Message-ID → session_id` 映射：

- 每当识别出一封 `echo`（自己外发邮件的回环）→ 用它携带的 message_id 建立映射
- `find_by_references` 未直接命中时，先查 `message_aliases` 再做二次定位

## 4. 出站定性（`outbox.kind`）

| kind | 触发时机 |
|---|---|
| `new_task_ack` | 新任务入队成功后的回执 |
| `resume_ack` | 继续任务入队后的回执 |
| `progress` | agent 运行中的进度更新（如 "Task still running" / 目录待确认） |
| `result` | 任务完成/失败的结果邮件 |
| `error` | 路由错误（前缀可识别但 agent 不支持、未安装等） |
| `hook` | 本地 agent 完成事件的报告 |
| `test` | SMTP 连接测试 |

出站邮件同时写入 `X-EMA-Kind` 头 + 正文标记行，便于入站侧识别自身邮件。

## 5. 硬性约束

- **绝不**因为 Message-ID 被改写就放行自己发出的邮件：必须走第 2 节特征判定 → 判为 `echo` 并丢弃
- **绝不**静默丢弃可识别的任务：新任务/继续任务必须产生显式回执（ack）或可读的错误邮件
- 映射表是幂等的：同一改写 ID 重复出现不重复建映射、不重复入队
- `kind` 是系统字段，写数据库时必须显式赋值，禁止留空或靠正文猜