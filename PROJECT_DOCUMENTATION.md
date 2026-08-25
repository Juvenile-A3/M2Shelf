# M²Shelf 项目说明

这是一份面向新开发者和新 AI Agent 的快速索引。稳定规则以 `AGENTS.md` 为首要入口；产品行为、当前实现和长期决策分别记录在 `docs/PRODUCT_SPEC.md`、`docs/PROJECT_CONTEXT.md` 和 `docs/DECISIONS.md`。

## 项目概览

M²Shelf 是 Windows 本地优先媒体资源浏览器。它读取用户选择的媒体目录，在应用自己的 SQLite 数据库中建立索引，并提供浏览、搜索、Bangumi 绑定、封面、标签、收藏夹、最近观看和外部播放器入口。

核心边界：媒体目录只读。显示名称、分类、绑定和整理操作只改变应用数据库，不改动真实文件。

当前源码版本为 `0.5.10`，主要用户闭环已经可用，并已加入可验证的稳定版更新客户端；产品仍以本机单用户和 Windows x64 为目标。

## 技术架构

- `src/`：React 19 + TypeScript 前端，Vite 构建；
- `src-tauri/`：Tauri 2 桌面壳和 Rust 后端；
- SQLite：资源索引、元数据和设置的唯一持久化数据库；
- Bangumi 官方 API：可选的动画与真人影视条目搜索和封面来源；
- GitHub Releases：固定 `latest.json` 的稳定更新清单，以及版本化 NSIS / Portable 资产；
- Windows 原生能力：外部程序启动、文件定位、窗口状态和多尺寸图标。

WebView 不直接访问数据库，也不拥有广泛文件系统或进程权限。前端通过 `src/lib/api.ts` 中的类型化 Tauri 命令调用 Rust。

## 代码地图

- `src/App.tsx`：应用启动、页面状态、导航快照和主要操作协调；
- `src/pages/`：全部资源、目录浏览、搜索、最近观看、收藏夹、详情、设置和首次启动页面；
- `src/components/`：海报、文件列表、菜单、批量编辑和对话框；其中 `PosterImage.tsx` 与 `src/lib/poster.ts` 为主列表和详情提供同一套有界、DPR 对齐的 Canvas 重采样；
- `src/components/Update*.tsx`：更新可用提示，以及紧凑的进度与安装确认界面；
- `src/lib/i18n.tsx`：四种界面语言的类型化文案；
- `src/types/media.ts`：前端领域模型；
- `src-tauri/src/scanner.rs`：只读扫描与分类输入；
- `src-tauri/src/db.rs`：迁移、查询和事务；
- `src-tauri/src/commands.rs`：前端可调用命令；
- `src-tauri/src/auto_match.rs`、`title_extractor.rs`、`bangumi.rs`：关键词、候选评分和官方 API；
- `src-tauri/src/cache.rs`：应用封面缓存；
- `src-tauri/src/player.rs`：外部播放器；
- `src-tauri/src/window_state.rs`：窗口尺寸恢复；
- `src-tauri/src/update.rs`：严格更新清单、稳定 SemVer、下载、SHA-256 和 Ed25519 校验；
- `src-tauri/src/portable_update.rs`：Portable helper 事务、健康回执与文件/数据库回滚；
- `src-tauri/src/single_instance.rs`：Windows 单实例；
- `src-tauri/src/bin/m2shelf_updater.rs`：Portable 更新 helper 和受控线下签名器；
- `src-tauri/migrations/`：兼容升级的 SQLite schema；
- `scripts/`：图标、验证、Windows 构建、线下签名和 fail-closed 发布脚本；
- `.github/workflows/windows-release.yml`：无生产私钥的 Windows 无签名候选构建。

## 主要流程

1. 启动：Rust 打开数据库并执行增量迁移，前端读取资源库、设置、排序和最近观看。
2. 扫描：Rust 只读遍历 Library Root，更新 Node、视频和附属资源索引；人工分类和应用元数据保留。
3. 匹配：从目录、视频和父目录提取结构化标题，搜索 Bangumi 动画 type 2 与真人影视 type 6 并评分；每个 Node 最多补全五个候选且全轮详情请求有总预算，只有高置信度候选可自动绑定。
4. 浏览：Rust 在同一 SQLite 读快照中批量 hydrate Node、Bangumi 绑定和标签，本地搜索也不逐项补查；前端按显示标题、标签和排序生成网格或列表，并以共用的 DPR Canvas 链路绘制主列表与详情封面。
5. 播放：Rust 以字面参数启动用户配置的播放器；成功启动后更新该 Node 的最近观看时间。
6. 更新：设置页将自动检查开关和手动检查按钮放在同一层级，只有有效新版本才显示详情；固定地址因旧/同版 Release 缺清单而 404 时只能报告无更新，不能产生下载候选。NSIS 启动已验证安装器；Portable 由可信 helper 在 ready handshake 后备份、替换、启动并检查健康，只有通过精确活动事务与 `Launched` 阶段认证的新版子进程可绕过更新锁，成功后的 `Completed` 清理可在下次启动续作，失败时回滚文件和 SQLite。

## 数据结构

主要表包括：`library_roots`、`nodes`、`media_files`、`resource_files`、`metadata_bindings`、`settings`、`scan_runs`、`tags`、`node_tags`、`watch_history`、`favorite_folders` 和 `node_favorite_folders`。

路径是索引身份。资源在磁盘外部移动或改名后，需要重新扫描；应用不会代替用户移动文件。所有 schema 修改必须增加 migration，不得丢弃现有数据库。

## 重要产品契约

- 不向媒体目录写入任何文件；
- 普通自动匹配不覆盖现有绑定或手工封面；
- 自动匹配只接受 Bangumi 动画 type 2 与真人影视 type 6，并限制查询、候选详情、响应体和重试；
- 封面失败不删除有效绑定；
- 非视频附件不单独成为作品；
- 人工分类、标签和收藏夹在重新扫描后保留；
- 语言、主题、窗口尺寸和排序持久化在应用数据库；
- 外部路径始终作为原生进程的字面参数，不拼接 shell 命令；
- 用户可见文字进入 i18n，界面使用系统字体；
- Logo 只从仓库指定母版按完整画布缩放生成。
- 主列表与详情封面共用 DPR 对齐、有界并发且可释放离屏 backing store 的 Canvas 重采样，缓存原图不变；
- 稳定更新只接受更高的规范 `major.minor.patch`、固定版本化资产 URL、声明长度、SHA-256 与绑定应用/版本/平台/大小/摘要的 Ed25519 签名；
- 启动自动检查不自动下载或安装；生产私钥只在 Agent 不可访问的独立离线环境使用，GitHub Actions 只能生成无签名候选，普通 provenance 不能代替受信 attestation 或独立构建摘要；
- Portable 更新在旧程序退出前完成包密封和 helper-ready，保持应用单实例，备份并以长度/SHA-256/SQLite 完整性复验快照，最后替换主程序，并要求精确版本健康回执及完整三秒存活；失败时只在确认新进程终止后执行可验证回滚并显示恢复状态。

## 已知边界

- 仅面向 Windows x64；
- 播放器由用户自行安装和配置；
- Bangumi 匹配依赖网络且可能需要人工纠正；
- 当前没有云同步、内置播放器或播放进度；
- Windows 代码签名与更新包 Ed25519 签名是不同机制；未做 Authenticode 代码签名的构建仍可能触发 Windows SmartScreen；
- `v0.5.8` 是 CI 失败的不可变历史 tag，`v0.5.9` 是最终修复前创建且未发布的不可变历史 tag；二者都没有 Release、资产或 `latest.json`。`0.5.10` 是内置更新器的首次公开引导版本，现有 `0.5.7` 用户必须手动安装一次。README 在 `v0.5.10` 正式发布前仍指向公开的 `0.5.7`。

## 30 分钟上手

1. 先读 `AGENTS.md`、`docs/PRODUCT_SPEC.md`、`docs/PROJECT_CONTEXT.md`、`docs/DECISIONS.md`。
2. 从 `src/App.tsx`、`src/lib/api.ts` 和 `src-tauri/src/commands.rs` 理解前后端调用。
3. 数据或扫描修改先读 `db.rs`、`scanner.rs` 和全部 migrations。
4. 匹配修改先读 `title_extractor.rs`、`auto_match.rs`、`bangumi.rs` 及其测试。
5. 更新相关修改还需读 `src-tauri/src/update.rs`、`portable_update.rs`、`bin/m2shelf_updater.rs` 和 `docs/UPDATE_RELEASE_PROCESS.md`。
6. 修改后运行 `AGENTS.md` 中的完整验证门禁；发布使用仓库脚本，不手工拼装、在 CI 中签名或替换已发布产物。
