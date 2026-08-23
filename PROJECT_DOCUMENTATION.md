# M²Shelf 项目说明

这是一份面向新开发者和新 AI Agent 的快速索引。稳定规则以 `AGENTS.md` 为首要入口；产品行为、当前实现和长期决策分别记录在 `docs/PRODUCT_SPEC.md`、`docs/PROJECT_CONTEXT.md` 和 `docs/DECISIONS.md`。

## 项目概览

M²Shelf 是 Windows 本地优先媒体资源浏览器。它读取用户选择的媒体目录，在应用自己的 SQLite 数据库中建立索引，并提供浏览、搜索、Bangumi 绑定、封面、标签、收藏夹、最近观看和外部播放器入口。

核心边界：媒体目录只读。显示名称、分类、绑定和整理操作只改变应用数据库，不改动真实文件。

当前源码版本为 `0.5.6`，主要用户闭环已经可用；产品仍以本机单用户和 Windows x64 为目标。

## 技术架构

- `src/`：React 19 + TypeScript 前端，Vite 构建；
- `src-tauri/`：Tauri 2 桌面壳和 Rust 后端；
- SQLite：资源索引、元数据和设置的唯一持久化数据库；
- Bangumi 官方 API：可选的动画条目搜索和封面来源；
- Windows 原生能力：外部程序启动、文件定位、窗口状态和多尺寸图标。

WebView 不直接访问数据库，也不拥有广泛文件系统或进程权限。前端通过 `src/lib/api.ts` 中的类型化 Tauri 命令调用 Rust。

## 代码地图

- `src/App.tsx`：应用启动、页面状态、导航快照和主要操作协调；
- `src/pages/`：全部资源、目录浏览、搜索、最近观看、收藏夹、详情、设置和首次启动页面；
- `src/components/`：海报、文件列表、菜单、批量编辑和对话框；
- `src/lib/i18n.tsx`：四种界面语言的类型化文案；
- `src/types/media.ts`：前端领域模型；
- `src-tauri/src/scanner.rs`：只读扫描与分类输入；
- `src-tauri/src/db.rs`：迁移、查询和事务；
- `src-tauri/src/commands.rs`：前端可调用命令；
- `src-tauri/src/auto_match.rs`、`title_extractor.rs`、`bangumi.rs`：关键词、候选评分和官方 API；
- `src-tauri/src/cache.rs`：应用封面缓存；
- `src-tauri/src/player.rs`：外部播放器；
- `src-tauri/src/window_state.rs`：窗口尺寸恢复；
- `src-tauri/migrations/`：兼容升级的 SQLite schema；
- `scripts/`：图标、验证和 Windows 发布脚本。

## 主要流程

1. 启动：Rust 打开数据库并执行增量迁移，前端读取资源库、设置、排序和最近观看。
2. 扫描：Rust 只读遍历 Library Root，更新 Node、视频和附属资源索引；人工分类和应用元数据保留。
3. 匹配：从目录、视频和父目录提取结构化标题，搜索 Bangumi 并评分；只有高置信度候选可自动绑定。
4. 浏览：前端读取索引，按显示标题、标签和排序生成网格或列表；详情页读取视频与附件。
5. 播放：Rust 以字面参数启动用户配置的播放器；成功启动后更新该 Node 的最近观看时间。

## 数据结构

主要表包括：`library_roots`、`nodes`、`media_files`、`resource_files`、`metadata_bindings`、`settings`、`scan_runs`、`tags`、`node_tags`、`watch_history`、`favorite_folders` 和 `node_favorite_folders`。

路径是索引身份。资源在磁盘外部移动或改名后，需要重新扫描；应用不会代替用户移动文件。所有 schema 修改必须增加 migration，不得丢弃现有数据库。

## 重要产品契约

- 不向媒体目录写入任何文件；
- 普通自动匹配不覆盖现有绑定或手工封面；
- 封面失败不删除有效绑定；
- 非视频附件不单独成为作品；
- 人工分类、标签和收藏夹在重新扫描后保留；
- 语言、主题、窗口尺寸和排序持久化在应用数据库；
- 外部路径始终作为原生进程的字面参数，不拼接 shell 命令；
- 用户可见文字进入 i18n，界面使用系统字体；
- Logo 只从仓库指定母版按完整画布缩放生成。

## 已知边界

- 仅面向 Windows x64；
- 播放器由用户自行安装和配置；
- Bangumi 匹配依赖网络且可能需要人工纠正；
- 当前没有云同步、内置播放器或播放进度；
- 未签名构建可能触发 Windows SmartScreen。

## 30 分钟上手

1. 先读 `AGENTS.md`、`docs/PRODUCT_SPEC.md`、`docs/PROJECT_CONTEXT.md`、`docs/DECISIONS.md`。
2. 从 `src/App.tsx`、`src/lib/api.ts` 和 `src-tauri/src/commands.rs` 理解前后端调用。
3. 数据或扫描修改先读 `db.rs`、`scanner.rs` 和全部 migrations。
4. 匹配修改先读 `title_extractor.rs`、`auto_match.rs`、`bangumi.rs` 及其测试。
5. 修改后运行 `AGENTS.md` 中的完整验证门禁；发布使用仓库脚本，不手工拼装产物。
