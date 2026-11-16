# QQ + Codex 真实邮件验收

日期：2026-11-16。关联 [Issue #34](https://github.com/tans/ema/issues/34)。测试标识：`EMA-QA-20261116-151129`。

使用本机已保存的 QQ SMTP/IMAP 凭据、真实 EMA 桌面进程和 Codex CLI 0.168.0。收发对象均为自己的 QQ 邮箱。通过 IMAP `BODY.PEEK[]` 获取实际收到的邮件，核对 MIME、正文、引用链、数据库与原生会话；没有使用模拟 Agent 答案。

## 验收范围

| 场景 | 验收内容 |
| --- | --- |
| 连接测试 `test` | 生产发送路径投递，QQ INBOX 实际收到 |
| 新任务 `new_task_ack` | 自己给自己发 `codex ...`，正确入队；回执发送后才执行 |
| 进度 `progress` | 真实 Codex 执行期间的一分钟通知，明确仍在运行 |
| 结果 `result` | 成功答案与真实执行失败原因均生成邮件；中文结果包含“中文成功” |
| 续接 `resume_ack` | 回复 QQ 改写后的 Message-ID，复用 EMA / Codex 原生会话 |
| 错误 `error` | 空正文返回 `EMPTY_BODY`；不存在的 Codex 命令返回 `AGENT_NOT_INSTALLED`，无任务执行 |
| 本地报告 `hook` | 真实 Codex notify，经 spool/collector 发信，保留指令、答案、目录及原生会话 ID |
| 普通邮件 | 无前缀、无已知引用 → `unrelated`，记录原因，无任务或回复 |
| 无效邮件 | 空正文、命令缺失 → `invalid`，保留原错误状态 |
| 防回环 | QQ 回投的外发邮件 → `echo`；剥离自定义头后，仅正文标记也能拦截 |
| 幂等 | 同一真实原生完成事件重放两次，仅生成一封报告 |
| 引用正文 | 回复含 `> EMA-Message...`，不误判 echo；引用内容不作为本次指令 |

修复后再次实收的七种出站类型均声明 `text/plain; charset=utf-8`。逐封核对 `X-EMA-Version=1`、`X-EMA-Kind`、原始 ID、首行机器标记和数据库正文（统一换行后相同）。

实收统计：26 封测试出站邮件，37 项检查全部通过。其中 test 2、new_task_ack 3、resume_ack 4、progress 3、result 7、error 3、hook 4；修复字符集后重新覆盖七种类型，共 13 封 UTF-8 邮件。测试期间另有一封用户本地 ALMA 报告，不计入本次 Codex 验收。

最终普通会话续接返回 `中文成功 / RESUME_UTF8_OK`，原生会话 ID 保持不变；Hook 续接返回 `HOOK_UTF8_OK / HOOK_RESUME_OK`，与原报告的原生会话 ID 一致。全部七个测试任务均满足回执发送时间不晚于执行开始时间（含两次真实失败）。

## 发现并修复

1. 白名单中的邮箱主人自发新任务，被旧的 own-address 分支忽略。移除此限制，继续由最优先的自身特征识别防回环。
2. 拒绝邮件错误地保留 `unrelated`，且空正文任务没有错误回信。显式写入类型和原因，为可识别空任务发送幂等错误回执。
3. 独立工作目录不是 Git 仓库，Codex 无法启动。仅为 `_agent_memory` 增加 `--skip-git-repo-check`，保留原 sandbox。
4. 外发正文缺少 UTF-8 MIME 声明。显式声明字符集，修复中文客户端解析风险。
5. Codex 原生 notify 使用 `input-messages` 数组，适配器漏读导致原始指令缺失。补齐该字段并验证真实报告。

新增三个回归检查：自发邮件到达 Codex 路由、空任务错误回执/类型/幂等、真实 notify 字段提取。对应检查通过；`cargo check --workspace`、`npm run build`、`git diff --check` 通过。

## 环境与边界

- 测试期间临时开启收件、15 秒轮询和一分钟进度提醒，保持只读权限；结束后逐字节核对并恢复原配置：任务开关关闭、30 秒轮询、10/30/60 分钟进度、原 `codex` 命令。桌面进程已使用原配置重新启动。
- 默认 Codex 的 WebSocket 连接存在重试延迟，回退 HTTPS 后已成功执行。后续内容验收也使用本机已配置的 `magpie` provider，通过临时命令包装器调用同一个真实 Codex；没有修改全局 Codex 配置。
- Codex 禁止续接时切换 provider。测试中跨 provider 的一次续接被明确拒绝，失败结果邮件正确投递；成功续接使用原 provider。
- 全局 Codex notify 已被其他工具占用，原配置保留。Hook 验收使用单次 CLI notify 覆盖；重复事件验证重放的是该次真实 CLI 产出的 JSON。
- 本次覆盖邮件与 Codex 链路，不代表其他 Agent、写权限执行、附件、所有邮箱服务商或全部安全拒绝分支已验收。
- 测试邮件保留在 QQ 与 EMA 历史中；凭据和原始邮箱地址未写入仓库。详细证据与原始配置/数据库备份留在本机私有备份目录。

收尾时连续两个轮询间隔未新增任务、出站邮件或别名；无 queued/running 任务。本次测试出站记录全部 sent，且已逐封在 QQ INBOX 找到。证据保存在应用数据目录 `backups/EMA-QA-20261116-151129/`（`audit.json`、`delivered.json`、稳定性快照及原配置/数据库备份）。
