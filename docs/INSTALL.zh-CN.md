# EMA Alpha 安装指南（零成本分发）

> EMA 目前属于**实验性 Alpha 版本**，并非经过 Apple 公证或 Windows 可信发布者认证的稳定软件。当前版本的邮件来源验证、危险操作审批以及真实邮箱 / Codex 的端到端验收仍待完善。仅在自己控制的测试邮箱和**非敏感工作目录**中使用，运行权限由 Agent 自身默认或用户配置决定，EMA 不覆盖权限或自动批准操作。

## 下载与验证

1. 普通用户可从 [EMA Releases](https://github.com/tans/ema/releases) 下载**公开的 Alpha 预览版**（启用自动更新后的新发布会标记为 Latest，而不是 GitHub Pre-release）。全部平台打包和校验成功后，GitHub Actions 自动公开预览版；若构建失败，则保留 Draft（不可公开下载）。开发者在版本 Tag 发布前，可到 [GitHub Actions](https://github.com/tans/ema/actions/workflows/build.yml) 的成功 CI 运行记录下载有效期 7 天的 `ema-macos-adhoc-dmg` 测试附件。
2. 选择对应系统安装包：
   - **macOS Apple Silicon**：ARM64 / aarch64 的 DMG
   - **macOS Intel**：x86_64 的 DMG
   - **Windows**：x64 安装程序（例如 `.msi` / `.exe`）
   - **Linux**：x64 `.AppImage` / `.deb`（以 Release 实际资产为准）
3. 同时下载 `SHA256SUMS`，核对文件是否完整。校验和只能检查下载内容是否与发布记录一致，**不能替代可靠的软件发布者身份验证**。

macOS / Linux（在下载目录执行）：

```bash
shasum -a 256 EMA_*.dmg
# Linux 也可以使用：
sha256sum EMA_*.AppImage
```

Windows PowerShell：

```powershell
Get-FileHash .\EMA_*.exe -Algorithm SHA256
```

将打印的哈希与 Release 的 `SHA256SUMS` 中对应文件逐一比较。

## macOS：免费 Ad-hoc 签名

EMA 的 macOS 安装包采用 Tauri 的 `signingIdentity: "-"` **Ad-hoc 签名**，无需付费 Apple Developer 会员。它**不代表 Apple Developer ID 签名，也没有 Apple 公证**。

1. 下载与你的 CPU 架构匹配的 `.dmg`。
2. 打开 DMG，将 EMA 拖到 Applications。
3. 正常尝试打开 EMA。如果 macOS 因开发者无法验证而阻止启动，请先确认下载来源及 SHA256。
4. 对确认可信的版本，可在 **系统设置 → 隐私与安全性** 中按 macOS 提供的 **“仍要打开 / Open Anyway”** 流程单独允许该应用。
5. 不要全局关闭 Gatekeeper，不要运行不明来源的脚本来批量移除隔离属性。

官方参考：[Apple 关于打开未认证应用的说明](https://support.apple.com/guide/mac-help/open-a-mac-app-from-an-unidentified-developer-mh40616/mac)、[Tauri Ad-hoc 签名](https://tauri.app/distribute/sign/macos/)。

## Windows：无付费证书

Windows 安装包暂时**没有 Authenticode 发布者签名**，可能出现“未知发布者”或 Microsoft Defender SmartScreen 警告。请先确认下载来自官方仓库、版本信息与校验和，不要为了安装 EMA 而关闭 SmartScreen 或杀毒软件。如果系统策略阻止运行，应暂停安装，等待正式签名发行版。

未来可在满足条件后申请 [SignPath Foundation](https://signpath.org/) 的开源免费签名服务；不承诺一定通过审核。

## Linux：AppImage / DEB

下载对应文件并核对校验和。如使用 AppImage：

```bash
chmod +x EMA_*.AppImage
./EMA_*.AppImage
```

部分发行版运行 AppImage 需要系统提供 FUSE 兼容层。也可优先使用对应发行版的 `.deb` 包。

## 首次使用

- 专用测试邮箱；不要让任何不可控用户知道具有执行权限的邮箱地址。
- 先检查本机 Agent 的权限配置并使用测试目录；旧 `workspace_write` 字段仅为兼容保留，不再影响执行，也无需 EMA-Code。续接使用原会话，无法在无头模式下完成的原生授权会返回错误。
- 本机需要先安装并登录 Codex CLI。
- 使用自己的测试邮箱先完成“发邮件 → Codex 阅读项目 → 收到结果邮件 → 回复继续”闭环。
- 不要让 EMA 在高敏感工作站或生产仓库中处理未验证邮件。

## 自动更新

已接入 Tauri v2 官方 Updater。首次需要手动安装包含此功能的版本；发布者配置签名 secrets 并发布带有 `latest.json` 的 Latest 版本后，才能实际获取更新。

启动后后台检查；没有更新或自动检查失败时不弹提示。发现新版本会在右下角 Toast 提醒，并保留“设置 → 软件更新”入口。也可以手动检查，点击下载后显示进度并验证签名，再确认安装和重启。失败可重试；运行中的邮件收发或 Agent 任务会阻止安装，请等待完成后重试。

补丁版本仅更新 Apple Silicon。Intel Mac、Windows 和 Linux 等待下一次 X.Y.0 全平台发布；Linux 自动更新仅适用于可写位置的 AppImage，DEB 请手动升级。更新签名不替代 Apple 公证或 Windows Authenticode 签名。发布设置和平台限制见 [发布文档](RELEASING.md)。
