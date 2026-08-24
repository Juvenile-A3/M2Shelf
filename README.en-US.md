<p align="center">
  <img src="./src-tauri/icons/128x128.png" width="96" height="96" alt="M²Shelf Logo">
</p>

<h1 align="center">M²Shelf</h1>

<p align="center"><strong>MORI MEDIA SHELF</strong></p>

<p align="center"><a href="./README.md">简体中文</a> · <a href="./README.ja-JP.md">日本語</a> · <a href="./README.ko-KR.md">한국어</a></p>

<p align="center">Built entirely with GPT, M²Shelf is a local-first browser for Windows media collections that treats source media as read-only.</p>

M²Shelf creates a separate index for anime, movies, and related resources stored on local drives, external drives, or mapped NAS folders. It provides poster-wall browsing, Bangumi metadata, tags, favorites, watch history, and shortcuts to external players.

**The app never moves, deletes, renames, or modifies source media files, and it does not require you to reorganize existing folders.**

## Key features

- Manage multiple media libraries and recursively scan folders at any depth;
- Browse all resources, individual libraries, and real folder hierarchies in poster or list view;
- Search local names, file names, multilingual Bangumi titles, and user tags;
- Automatically classify works, series, and other resources while preserving manual classifications;
- Automatically match high-confidence Bangumi results, with manual search, correction, and cover retry options;
- View videos alongside related subtitles, images, audio, documents, archives, and other files;
- Play media with a configured external player and reveal files in Windows File Explorer;
- Organize items with tags, one-level named favorite folders, and batch edit mode;
- Show recently watched works after M²Shelf successfully starts playback;
- Support Simplified Chinese, English, Japanese, and Korean;
- Support system, light, and dark themes;
- Remember window size, sort choices, and in-session positions for each browsing section;
- Use a custom location for the app-owned cover cache.

## Local-first design and privacy

Media directories are always treated as read-only. M²Shelf stores its index, display names, Bangumi bindings, tags, favorites, watch history, and settings in its own SQLite database. Covers are stored in the app cache.

Bangumi search and cover downloads require an internet connection. Browsing the local index and opening local files do not depend on Bangumi. The project requires no media server or cloud account and never uploads media files to a remote service.

## Download

Current version: **M²Shelf 0.5.7** (Windows x64)

- [Download the Portable build](https://github.com/Undermori/M2Shelf/releases/download/v0.5.7/M2Shelf-Portable-0.5.7-x64.zip)
- [View the latest release](https://github.com/Undermori/M2Shelf/releases/latest)
- [View all releases](https://github.com/Undermori/M2Shelf/releases)

Using the Portable build:

1. Extract the entire ZIP file; do not run the app from inside the archive;
2. Double-click `M2Shelf.exe`;
3. Add and scan a media directory;
4. Configure an external player path if needed.

Portable means the app itself does not require installation. The database, settings, and default cover cache are still stored in the Windows application data directory. The current build is not code-signed, so Windows SmartScreen may show an “Unknown publisher” warning. The interface requires Microsoft Edge WebView2 Runtime.

## Current scope

M²Shelf currently does not provide a built-in player, online streaming, transcoding, a media server, account sync, automatic subtitles, or playback-resume progress. It also never automatically moves or renames media files.

## Development

Stack: Tauri 2, Rust, React 19, TypeScript, Vite, and SQLite.

```powershell
npm install
npm run tauri dev
```

Checks before committing:

```powershell
npm run typecheck
npm run build
npm run validate
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --locked -- -D warnings
```

Official Windows artifacts are built with `scripts/build_windows_release.ps1`.

## Project documents

- [Development rules](./AGENTS.md)
- [Product specification](./docs/PRODUCT_SPEC.md)
- [Current implementation](./docs/PROJECT_CONTEXT.md)
- [Long-term decisions](./docs/DECISIONS.md)
- [Quick project guide](./PROJECT_DOCUMENTATION.md)

## Author

- [森下Undermori · Bilibili](https://space.bilibili.com/2903441)
- [Undermori · X](https://x.com/f_undermori)
