# M²Shelf 稳定更新发布流程

本文记录 `0.5.8` 起的 Windows 稳定更新发布契约。它只包含可公开的操作规则；生产私钥、DPAPI 文件、独立指纹、个人路径和账号凭据不得写入仓库、Issue、Actions、Release 或日志。

## 1. 信任模型

- 客户端只读取 `https://github.com/Undermori/M2Shelf/releases/latest/download/latest.json`；
- `latest.json` 内的资产 URL 必须固定到 `v{version}` Release，不能指向 `latest`、任意主机或可变文件名；
- 版本只允许严格高于当前版本的规范 `major.minor.patch`；
- 每个资产同时校验固定文件名、长度、SHA-256 和 Ed25519；签名绑定应用 ID、版本、平台、长度和摘要；
- GitHub Actions 不接触生产私钥，只构建无签名候选；
- 生产 seed 只存于独立、离线、日常开发工具和 Agent 均无法访问的签名环境。CurrentUser DPAPI 只能保护静态文件，不能隔离同一 Windows 用户下的进程，因此开发机上的 DPAPI 文件只能用于开发验收，不能单独充当生产私钥边界；
- 已发布 Release 视为不可变。修复错误发布必须提升版本，不能替换原资产。

## 2. 发布前准备

1. 确认 `package.json`、lockfile、`src-tauri/Cargo.toml`、Cargo lockfile 和 `src-tauri/tauri.conf.json` 版本完全一致，且为稳定规范 SemVer。
2. 更新四语言 release notes，并确认所有长期文档与源码一致。
3. 在干净工作树运行 `AGENTS.md` 的完整质量门禁。
4. 使用 `scripts/build_windows_release.ps1 -Bundles nsis` 与 `scripts/build_portable.ps1` 做一次本地候选验证；检查版本、x64、图标、隐私扫描、Portable 五文件集合、helper identity 和启动。
5. 提交并推送目标 commit，在该 commit 创建唯一 tag `v{version}`，再推送 tag。不要移动或复用发布 tag。
6. 发布前审计完整可达 Git 历史的 author/committer；公开仓库只允许 GitHub noreply 地址。发现真实邮箱时必须先按经用户确认的历史清理流程处理并复核远端，再创建 release tag。

`0.5.8` 是首次 updater 引导版。`0.5.7` 及更早客户端无法通过不存在的内置 updater 获取它，Release 文案必须明确要求手动安装一次。

## 3. 获取 CI 无签名候选

tag 会触发 `.github/workflows/windows-release.yml`。该 workflow 使用固定 action commit，执行完整门禁，构建 NSIS、Portable 和 helper，并上传短期保存的：

- `M2Shelf-Portable-{version}-x64.zip`；
- `M2Shelf-Portable-{version}-x64.zip.sha256`；
- `M2Shelf-Setup-{version}-x64.exe`；
- `M2Shelf-Setup-{version}-x64.exe.sha256`；
- `candidate-provenance.json`。

下载候选到仓库外或忽略的本地目录，保留原文件名与 provenance。`candidate-provenance.json` 是便于核对的 CI 元数据，不是密码学来源证明，不能单独授权生产签名。只接受 tag 触发、`tagExists=true`，且 repository、version、tag、commit 和资产摘要均与本地目标一致的候选；此外必须通过受信 Artifact Attestation，或在隔离环境从已验证 tag 独立构建并记录两个资产摘要。正式签名必须使用这条独立路径得到的摘要。

## 4. 线下签名

发布负责人应预先拥有七项仓库外、独立核对的输入：

1. CurrentUser DPAPI 保护的 32 字节 Ed25519 seed；
2. 已审查的 `M2ShelfUpdater.exe` 签名 helper；
3. 在不同步骤、独立保存的该 helper SHA-256 指纹；
4. 已审查 `generate_update_manifest.ps1` 的 SHA-256 指纹；
5. 通过受信 attestation 或隔离环境独立构建得到的 Portable SHA-256；
6. 同一路径得到的 NSIS SHA-256；
7. 四语言 release notes。

生产操作必须在独立签名机/用户上进行，且该环境不得向 Codex、其他 Agent、日常浏览器或普通开发进程提供命令执行能力。签名脚本和 helper 使用仓库外只读受信副本；首次建立 0.5.8 信任根前应在该环境生成并嵌入最终公钥。开发机 DPAPI seed 只允许验证流程，不得发布为生产信任根。

不要在命令行直接设置或回显私钥。使用 wrapper，使明文 seed 只短暂存在于当前 PowerShell 进程环境和内存中：

```powershell
pwsh ./scripts/sign_update_offline.ps1 `
  -CandidateDirectory $candidateDirectory `
  -EncryptedSeedPath $encryptedSeedPath `
  -UpdaterPath $trustedUpdaterPath `
  -TrustedSignerSha256 $trustedUpdaterSha256 `
  -TrustedManifestGeneratorSha256 $trustedManifestGeneratorSha256 `
  -TrustedPortableSha256 $trustedPortableSha256 `
  -TrustedNsisSha256 $trustedNsisSha256 `
  -NotesPath $localizedNotesJson
```

`-NotesPath` 可省略；正式发布必须提供恰好包含 `zh-CN`、`en-US`、`ja-JP`、`ko-KR` 的非空 JSON。wrapper 会在解密 seed 前核对 manifest generator、helper 和两个候选的独立指纹，再核对 app ID、版本、嵌入公钥和固定资产名，并生成：

- 两个资产各自的 `.sha256` 和 `.sig`；
- 严格 schema 的 `latest.json`。

签名后不要修改任何候选、sidecar、manifest 或 provenance。不要把签名目录提交到 Git。

## 5. Fail-closed 验证与发布

发布必须在正常 Git worktree 中进行，并满足：工作树干净、HEAD 等于本地 `v{version}`、远端 tag 指向同一不可变对象、候选 provenance 来自该 tag/commit。

先运行只读验证：

```powershell
pwsh ./scripts/publish_signed_release.ps1 `
  -CandidateDirectory $candidateDirectory `
  -VerifierPath $trustedUpdaterPath `
  -TrustedVerifierSha256 $trustedUpdaterSha256 `
  -ReleaseNotesPath $releaseNotesMarkdown
```

输出明确说明验证通过且没有修改 GitHub 后，才运行：

```powershell
pwsh ./scripts/publish_signed_release.ps1 `
  -CandidateDirectory $candidateDirectory `
  -VerifierPath $trustedUpdaterPath `
  -TrustedVerifierSha256 $trustedUpdaterSha256 `
  -ReleaseNotesPath $releaseNotesMarkdown `
  -Publish
```

发布脚本会先核对独立保存的 verifier SHA-256、app ID、版本和嵌入公钥，并调用其 `verify` 命令对两个资产执行真实 Ed25519 验签；然后锁定本地输入，拒绝已有 Release，创建 draft，上传并要求每个远端资产同时精确匹配名称、大小和 GitHub 报告的 SHA-256 digest，重新确认 tag，最后才公开。正式集合恰好包含：两个安装资产、两个 SHA-256 sidecar、两个 Ed25519 signature sidecar、`latest.json` 和 `candidate-provenance.json`。任一检查失败即停止；若已经建立 draft，它保持为 draft，必须先查明原因，不能绕过脚本直接公开。

## 6. 发布后验证

1. 核对 Release tag、commit、八个资产名、大小和公开 SHA-256；
2. 下载公开的 `latest.json`，确认版本、UTC 时间、四语言说明及两个 URL 固定到刚发布的 tag；
3. 在上一稳定版的 NSIS 和 Portable 环境各检查一次：发现更新、明确下载、完整性/签名验证、安装与重启；
4. Portable 还需验证 helper-ready 后才退出旧程序、单实例、数据库保留、精确版本健康回执和完整三秒存活；用测试构造的失败场景确认文件/SQLite 自动回滚及两类恢复提示；
5. 确认 Library Root 内容和时间戳未变化，发布资产及解压 Portable 不含构建者个人路径；
6. 只有正式 `v0.5.8` 已发布后，才能把 README 的公开版本与下载链接从 `0.5.7` 更新到 `0.5.8`。

更新包的 Ed25519 签名保护 M²Shelf updater 供应链，不等同于 Windows Authenticode。若没有 Authenticode，Windows 仍可能显示 SmartScreen 提示。
