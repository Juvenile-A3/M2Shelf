<p align="center">
  <img src="./src-tauri/icons/128x128.png" width="96" height="96" alt="M²Shelf Logo">
</p>

<h1 align="center">M²Shelf</h1>

<p align="center"><strong>MORI MEDIA SHELF</strong></p>

<p align="center"><a href="./README.en-US.md">English</a> · <a href="./README.ja-JP.md">日本語</a> · <a href="./README.ko-KR.md">한국어</a></p>

<p align="center">完全由 GPT 完成，面向 Windows 本地媒体收藏的本地优先、媒体源只读浏览器。</p>

M²Shelf 把本地硬盘、移动硬盘或 NAS 映射目录中的动画、电影及相关资源建立为独立索引，提供海报墙浏览、Bangumi 元数据、标签、收藏夹、观看记录和外部播放器入口。

简单来说，可以让你的动画收藏从资源管理器里不同语言、不同压制方式、不同字幕组等不易识别的长文件名一键变成如图所示的清晰瀑布流：

<img width="1427" height="888" alt="微信图片_20260823213336_62_30" src="https://github.com/user-attachments/assets/c09f2e18-aef0-4e13-ae1a-b5a7825bc3dc" />

在动画的详情页也依旧可以看到文件名显示，并可一键在资源管理器中打开。

**软件不会移动、删除、重命名或修改源媒体文件，也不要求整理现有目录。**

## 主要功能

- 管理多个媒体资源库并递归扫描任意深度目录；
- 使用海报墙或列表浏览全部资源、单个资源库和真实目录层级；
- 搜索本地名称、文件名、Bangumi 多语言标题和用户标签；
- 自动识别作品、系列和其他资源，并保留人工分类；
- 高置信度自动匹配 Bangumi，亦可手动搜索、纠正或重试封面；
- 展示视频，以及字幕、图片、音频、文档、压缩包等附属资源；
- 使用已配置的外部播放器播放，并在 Windows 资源管理器中定位文件；
- 使用标签、一层命名收藏夹和编辑模式批量整理；
- 按最近时间展示由 M²Shelf 成功启动播放的作品；
- 支持简体中文、English、日本語、한국어；
- 支持跟随系统、亮色和暗色主题；
- 记忆窗口尺寸、排序选择和各浏览分区的会话内位置；
- 支持自定义应用封面缓存位置。

## 本地优先与隐私

媒体目录始终视为只读。M²Shelf 的索引、显示名称、Bangumi 绑定、标签、收藏夹、观看记录和设置保存在应用自己的 SQLite 数据库中，封面保存在应用缓存中。

Bangumi 搜索和封面下载需要联网；本地索引浏览与打开本地文件不依赖 Bangumi。项目不需要媒体服务器或云端账号，也不会把媒体文件上传到远程服务。

## 下载

当前版本：**M²Shelf 0.5.11**（Windows x64）

- [下载 Portable 免安装版](https://github.com/Undermori/M2Shelf/releases/download/v0.5.11/M2Shelf-Portable-0.5.11-x64.zip)
- [查看最新 Release](https://github.com/Undermori/M2Shelf/releases/latest)
- [查看全部版本](https://github.com/Undermori/M2Shelf/releases)

Portable 使用方法：

1. 完整解压 ZIP，不要在压缩包内直接运行；
2. 双击 `M2Shelf.exe`；
3. 添加媒体目录并扫描；
4. 按需设置外部播放器路径。

Portable 表示应用本体无需安装。数据库、设置和默认封面缓存仍会写入 Windows 应用数据目录。当前构建未进行代码签名，Windows SmartScreen 可能提示“未知发布者”；运行界面依赖 Microsoft Edge WebView2 Runtime。

## 当前边界

M²Shelf 当前不提供内置播放器、在线视频、转码、媒体服务器、账号同步、自动字幕、续播进度，也不会自动移动或重命名媒体文件。

## 开发

技术栈：Tauri 2、Rust、React 19、TypeScript、Vite 和 SQLite。

```powershell
npm install
npm run tauri dev
```

提交前验证：

```powershell
npm run typecheck
npm run build
npm run validate
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --locked -- -D warnings
```

面向 Windows 的正式产物通过 `scripts/build_windows_release.ps1` 构建。

## 项目文档

- [开发规则](./AGENTS.md)
- [产品规格](./docs/PRODUCT_SPEC.md)
- [当前实现](./docs/PROJECT_CONTEXT.md)
- [长期决策](./docs/DECISIONS.md)
- [快速项目说明](./PROJECT_DOCUMENTATION.md)

## 作者

- [森下Undermori · Bilibili](https://space.bilibili.com/2903441)
- [Undermori · X](https://x.com/f_undermori)
