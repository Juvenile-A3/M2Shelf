# M²Shelf 当前项目上下文

本文记录仓库当前实现，供新开发者和新 Agent 定位代码。产品行为以 `PRODUCT_SPEC.md` 为准，稳定约束以根目录 `AGENTS.md` 为准。

## 当前状态

- 版本：`0.5.7`
- 目标：Windows x64 桌面应用
- 前端：React 19、TypeScript 5.8、Vite 6
- 客户端：Tauri 2、Rust 2021
- 数据：SQLite（`rusqlite` bundled）
- 网络：`reqwest` + rustls native roots，使用系统代理
- 主要外部服务：Bangumi 官方 API 和封面主机

仓库没有独立服务器。所有数据库访问在 Rust 中完成，前端通过类型化 Tauri 命令通信。

## 入口和目录

### 前端 `src/`

- `main.tsx`：React 入口和 i18n Provider；
- `App.tsx`：启动装载、页面切换、扫描事件、导航历史和跨页面操作；
- `types/media.ts`：前端 DTO、语言、主题、分类、排序和设置类型；
- `lib/api.ts`：唯一的 Tauri invoke 封装；
- `lib/i18n.tsx`：`zh-CN`、`en-US`、`ja-JP`、`ko-KR` 文案及标题选择；
- `lib/poster.ts`、`hooks/useCoverDataUrl.ts`：海报布局和缓存图加载；主列表与详情页共用海报框比例，网格用整数像素列宽并只通过 IntersectionObserver 启动延迟加载；
- `pages/`：Onboarding、All Resources、Browse、Search、Recently Watched、Favorites、Work Detail、Settings；
- `components/`：海报网格、文件列表、标签筛选、上下文菜单、编辑模式和对话框；
- `styles.css`：语义主题变量、布局和响应式样式。

前端没有 React Router 或独立全局状态库。`App.tsx` 维护轻量页面状态，并结合 `window.history` 与目的地快照实现返回和滚动恢复。扫描事件监听在应用生命周期内只注册一次，通过最新回调引用避免页面状态变化导致重复监听；异步注册结束后会用原生扫描快照补偿注册窗口内的终态事件。

### Rust `src-tauri/src/`

- `main.rs` / `lib.rs`：Tauri 入口、状态初始化、迁移、命令注册和窗口生命周期；
- `models.rs`：Rust 领域模型和序列化 DTO；
- `commands.rs`：资源库、浏览、搜索、设置、匹配、缓存、标签、收藏夹和原生操作命令；
- `db.rs`：连接、migration、查询、事务、自然排序和持久化设置；
- `scanner.rs`：只读目录遍历、视频/附件索引、BDMV 处理、分类和旧行清理；
- `title_extractor.rs`：标题、季度、年份、字幕组和文件噪声提取；
- `auto_match.rs`：有界多查询、候选合并、评分和置信度门禁；
- `bangumi.rs`：官方搜索、Subject 获取和封面请求；
- `cache.rs`：应用封面缓存校验、同目录原子提交、读写/清理屏障与拥有文件清理；
- `player.rs`：播放器测试和字面参数启动；
- `window_state.rs`：窗口尺寸校验、恢复和保存。

### 配置与脚本

- `src-tauri/tauri.conf.json`：窗口、CSP、asset scope、包标识和图标；
- `src-tauri/capabilities/default.json`：最小 Tauri 权限；
- `src-tauri/icons/`：用户确认母版及派生的 PNG、ICO、SVG；
- `scripts/validate_project.py`：跨源码和构建契约验证；
- `scripts/build_windows_release.ps1`：公开 Windows 构建、Portable 组装和隐私检查；
- `scripts/build_portable.ps1`：Portable 目录和 ZIP；
- `scripts/generate_icons.ps1`、`build_brand_assets.ps1`：从固定母版生成图标。

## 数据库

`db.rs` 在应用启动时按序执行 `src-tauri/migrations/`，并记录 schema 版本。迁移必须只增量升级。

当前 migrations：

1. `0001_initial.sql`：`library_roots`、`nodes`、`media_files`；
2. `0002_mvp.sql`：视频统计、`metadata_bindings`、`settings`、`scan_runs`；
3. `0003_resources_and_cover_status.sql`：`resource_files` 和封面错误；
4. `0004_multilingual_metadata.sql`：英、日、韩 Bangumi 标题；
5. `0005_user_tags.sql`：`tags`、`node_tags`；
6. `0006_watch_history.sql`：`watch_history`；
7. `0007_favorite_folders.sql`：`favorite_folders`、`node_favorite_folders`。

关键关系：

- 一个 Library Root 有多棵 Node 树；
- Node 通过 `parent_node_id` 形成同 Root 层级；
- 视频和附件分别落入 `media_files` 与 `resource_files`；
- 每个 Node 最多一个 Bangumi 绑定；
- 标签、收藏夹通过关联表实现多对多；
- 最近观看每个 Node 一行，删除 Node 时外键级联；
- `settings` 同时保存 AppSettings、窗口尺寸和分作用域排序键。

## 核心调用流程

### 启动

Rust 以隐藏状态创建主窗口，创建应用数据目录、打开 SQLite、执行 migrations，并在首次显示前恢复经过 DPI/工作区校验的窗口尺寸。前端先独立应用持久化语言和主题，在 React 首屏提交后通知原生窗口显示；其余 bootstrap、资源库、排序、扫描状态、全部资源和最近观看继续并行装载。

### 扫描

前端调用扫描命令并监听进度事件。`scanner.rs` 只读遍历目录，`db.rs` 在事务中更新 Node、文件、计数和分类。完成后，未绑定且合格的 Node 可进入自动匹配；人工分类和应用元数据不被普通扫描覆盖。

### 自动匹配

`title_extractor.rs` 生成结构化证据，`auto_match.rs` 最多发起三个搜索并公平合并候选，依据多语言标题、季度、年份和类型评分。只有高置信度且领先分差足够的 Anime Subject 写入绑定。普通路径不替换绑定或手工封面。

### 封面

Bangumi 封面下载到活动应用缓存，Node 保存实际缓存路径和失败原因。本地手工封面也复制到缓存。两类写入均使用目标目录内 UUID 临时文件并原子替换，失败时保留旧成品。普通读取/写入共享缓存操作屏障，显式清理独占屏障并在扫描或自动匹配运行时被后端拒绝，因此文件变化和 SQLite 路径更新不会交错。切换缓存目录只影响新写入；旧路径继续可读，清理范围仍受应用拥有目录与文件名限制。

### 浏览与导航

Rust 返回已 hydrate 的 Node DTO；列表所需的 Bangumi 绑定与用户标签按最多 500 个 Node 分块批量查询，避免全部资源和目录浏览退化为每项两次附加 SQL。前端按当前语言选择标题，封面 IPC 保持 4 路并发，并用 128 项/约 32 MiB 字符预算的 LRU 缓存已解码 data URL。全部资源、搜索、最近观看、收藏夹和每个 Root 各有会话快照。历史返回恢复快照；资源库与收藏夹恢复会重新查询当前行，避免把旧业务数据写回 UI。

### 播放

`player.rs` 以程序路径和媒体路径的独立字面参数启动播放器。只有成功 spawn 后，`db.rs` 才 upsert 最近观看记录。

## 持久化设置

`AppSettings` 保存播放器、默认视图、视频扩展名、Bangumi 开关、封面缓存、语言和主题。设置页采用序列化自动保存。

窗口尺寸使用独立 `settings` 键，由原生生命周期保存；原生窗口隐藏创建，恢复尺寸和前端主题首帧完成后再显示。全部资源、资源库浏览和收藏夹排序也使用独立键。完整 AppSettings 更新不得覆盖这些键。

## 安全边界

- Library Root 只读；应用写入仅限 SQLite、应用缓存和构建输出；
- Library Root 添加命令先 canonicalize 并检查重叠，数据库在 `BEGIN IMMEDIATE` 事务内再次拒绝等于、祖先或子孙 Root；扫描命令与 worker 入口还会复核登记路径和旧数据库中的重叠 Root，防止 Node 归属漂移；
- 扫描会对根、局部目标、递归目录和文件重新 canonicalize，并拒绝 Library Root 外的链接或重解析目标；
- Tauri capability 只开放所需能力，前端无直接 SQL；
- CSP 限制资源和连接来源；
- Bangumi 请求有官方主机白名单、TLS、超时和结果上限；
- 文件和播放器路径不经过 shell；
- 自定义缓存不可位于 Library Root；
- 手工封面在原子写入缓存前校验 15 MiB 上限、格式签名和像素尺寸；播放器测试有 5 秒超时；持久化 IPC 文本有后端长度上限；
- 批量标签、收藏夹和分类操作先验证 Node 集并事务提交；
- 仓库不得包含密钥、个人路径、真实索引数据库或私密截图。

## 修改路由

- UI / 页面：先看对应 `pages/`、`components/`，再看 `App.tsx`；
- 前后端接口：同步改 `types/media.ts`、`lib/api.ts`、`models.rs`、`commands.rs`；
- 数据结构：新增 migration，并补 `db.rs` 兼容测试；
- 扫描：检查完整/局部扫描、清理、取消、分类和只读边界；
- 匹配：同步检查 extractor、scorer、Bangumi client 和现有测试；
- i18n：四种语言一起更新；主题：检查 light、dark、system；
- 发布：使用 `scripts/build_windows_release.ps1`，不要手工复制目标文件。

## 验证

```text
npm run typecheck
npm run build
npm run validate
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --locked -- -D warnings
```

发布还需验证 Portable 的版本、架构、多帧图标、校验和、隐私扫描和启动。生成目录、依赖目录、数据库副本和本机缓存不属于源码，不能提交。

## 当前本地发布状态

- `0.5.7` Windows x64 Release 已于 2026-08-24 通过正式脚本构建；
- 本地 Portable：`bundle/M2Shelf-Portable-0.5.7-x64.zip`；
- SHA-256：`3DF766478A9E6733DDCF36FFA1BDA59E8A5BCD0676AB32C2807A1A0AD15CCA5F`；
- 已验证压缩包与内层校验和、版本元数据、x64 PE、隐私标记和短时启动；当前尚未提交、推送或发布到 GitHub。
