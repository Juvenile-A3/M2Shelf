# M²Shelf Agent Guide

This file contains stable rules for every coding agent that works in this repository. It is not a copy of any one delivery request.

## Read first

For a fresh account, machine, or agent, read in this order before changing code:

1. `AGENTS.md`
2. `docs/PRODUCT_SPEC.md` — intended product behavior
3. `docs/PROJECT_CONTEXT.md` — the implementation that exists now
4. `docs/DECISIONS.md` — decisions that must not be silently reversed
5. the user's current request
6. the code and migrations related to that request

Repository documents, code, and tests are the durable source of context. Do not assume access to an earlier chat, Codex Memory, or another agent's private state.

## Product identity

- User-facing brand: **M²Shelf**
- Technical / ASCII name: **M2Shelf**
- Subtitle: **MORI MEDIA SHELF**
- Product: a Windows, local-first media collection browser and manager
- Use `M²Shelf` in ordinary UI. `M2Shelf` is allowed for executable, archive, package, identifier, and other technical names.

## Non-negotiable product rules

- Treat every user media root as read-only. Never rename, move, delete, rewrite, or create files in it.
- A display-name change updates M²Shelf's SQLite data only; it never changes a real file or folder name.
- Bangumi data belongs to the metadata and presentation layer. It never renames source media.
- A completed scan or an explicit “match existing resources” action may auto-bind an unbound, Bangumi-eligible Node only after structured title evidence, at most three official search queries, multilingual candidate scoring, and the configured confidence/margin gates select a high-confidence Anime Subject without a strong season/year/kind conflict. Merge the bounded multi-query candidate pool fairly by provider rank instead of letting an earlier query consume it. Medium/low confidence never enters `metadata_bindings`. Never replace an existing binding or manual cover in the ordinary automatic path, and keep manual search available for correction. Only the user's explicit “rematch selected” action may replace an existing Bangumi binding after the same high-confidence gate; it still never replaces a manual cover. A non-manually-classified supplementary child such as SP/OVA/Extras under a parent with direct videos is structurally excluded from automatic candidates, while manual Bangumi binding remains available.
- Keep a valid Bangumi binding when cover retrieval fails, expose that failure, and allow an explicit retry.
- Store downloaded and manually selected covers only in the application-owned cache.
- Non-video files are resources attached to a Node. They do not become works, Bangumi candidates, or project-count entries merely because they exist.
- Work / Container / Mixed classification remains video-distribution based and must preserve manual overrides across rescans.
- Show the player feature as “播放器” in UI. The compatible internal setting name `mpv_path` may remain, and mpv may be mentioned as an example.
- Use the Windows/system UI font stack. Do not bundle or redistribute `.ttf`, `.otf`, or `.ttc` files.
- The canonical active M² artwork is `src-tauri/icons/icon-source.png`, created from the user-provided `logo-input-original.png` by making only the four black corner backgrounds transparent. Keep that prepared source byte-for-byte unchanged and derive active PNG / ICO sizes only by full-frame proportional scaling: do not crop, trace, redraw, recolor, sharpen, rearrange, or alter its rounded outline, gradient, glow, or lettering.
- Keep user-facing copy in the typed i18n resources. The supported interface locales are `zh-CN`, `en-US`, `ja-JP`, and `ko-KR`; changing locale must never change source names or paths.
- A bound title uses the current locale, then Chinese, the Bangumi main title, the database display name, and the real folder name in that order. Container series suffixes are localized presentation only.
- Theme changes use shared semantic CSS variables and support `system`, `light`, and `dark`; do not implement dark mode as a color inversion.
- A custom cover cache must be outside every Library Root and must never turn cache cleanup into deletion of unrelated user files. Keep old cached covers readable after switching roots; do not silently migrate or delete them.
- User tags are application-owned many-to-many metadata. Preserve them across rescans, keep single and batch tag edits out of source media trees, and keep user tags visually distinct from the three automatic card labels: Work, Series, and Other resources. Batch edit operations must validate and commit their selected Node set transactionally.
- Favorites are application-owned, one-level named collections beneath the sidebar Favorites entry. A Node may belong to multiple favorites; preserve membership across rescans, remove it only through explicit user actions or normal Node foreign-key cleanup, and validate batch membership changes transactionally without writing into media roots.
- Recently watched is application-owned Node history. Record it only after the configured player process is successfully spawned, keep one latest timestamp per Node, remove it only through normal SQLite foreign-key cleanup when that Node index is removed, and never infer it from or write it into source media.
- Opening a new detail starts at the top. Returning through the app, `Alt+Left`, or WebView/system history restores the previous scroll, filter, sort, and grid/list state when a snapshot exists. Sidebar browsing destinations keep separate in-session snapshots (`all`, `search`, `recent`, `favorites`, and each Library Root); switching away and back restores that destination instead of resetting it, while History remains a separate chronological trail. Library and Favorites section restores must rehydrate current indexed rows so navigation state cannot overwrite newer scans or metadata edits.
- All Resources, Library Browse, and Favorites persist their latest sort choices independently in the application-owned SQLite `settings` key/value table. These navigation preferences stay separate from complete Settings-page `AppSettings` snapshots and never modify Node metadata or source media.
- Poster artwork must use normal browser interpolation without translating the image-bearing frame onto a transformed compositor layer. Clearly landscape or near-square provider artwork is contained inside the portrait frame instead of being aggressively cropped and upscaled; ordinary portrait covers remain edge-to-edge.
- Keep existing behavior compatible unless a current requirement explicitly changes it. Avoid unrelated large refactors.

## Engineering constraints

- Inspect the real data model and call paths before implementing a document's example literally.
- Every database schema change requires an additive migration that upgrades existing databases without discarding bindings, custom names, covers, settings, or manual type overrides.
- Scanner changes must be checked against classification, partial scan/ancestor refresh, stale-row cleanup, cancellation, Unicode paths, and the source-read-only boundary.
- All database access stays in Rust; the WebView uses typed Tauri commands and has no direct SQL or broad filesystem/process permission.
- Pass paths to native processes as literal arguments. Never interpolate media paths into a shell command.
- On Windows, revealing a file means selecting that exact file through the native Shell API; Unicode, spaces, brackets, and other path characters must remain literal. Keep the explicit multi-frame ICO and Per-Monitor V2 manifest wiring intact. The 256×256 ICO entry must remain first because Tauri decodes the first frame as its runtime default window/taskbar icon; retain the remaining 128/64/48/32/24/16 frames for Windows resource selection.
- Keep Bangumi network access explicit, TLS-verified, bounded, and restricted to approved official hosts.
- UI changes should extend the established layout and accessibility patterns; check both the default window and the minimum supported window.
- For i18n/theme changes, check every supported locale, both resolved themes, native dialog labels, ARIA/title text, and system-theme changes while the app is open.
- When user-facing copy changes, audit the repository for the `M²Shelf` brand and generic “播放器” terminology.
- Do not commit secrets, access tokens, personal filesystem paths, private chat transcripts, or full copies of one-off requirement documents to project context files.
- Keep public repository history privacy-safe: use a GitHub `noreply` author address, and do not commit screenshots that expose personal paths, library contents, account details, or machine-specific data.
- Build public Windows artifacts through `scripts/build_windows_release.ps1` and verify that the final executable and unpacked Portable payload do not contain the builder's user-profile or workspace path before upload.

## Required verification

After relevant changes, run the applicable full gate and fix failures before handoff:

```text
npm run typecheck
npm run build
npm run validate
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --locked -- -D warnings
```

For a release, also build the Tauri binary, assemble the versioned Windows x64 Portable archive, verify its executable/version/icon/checksum, and smoke-test launch. Document any test that genuinely cannot run; never leave a known compile or type error for the next agent.

## Keep durable context current

At the end of each development round, decide whether the work changed stable behavior, architecture, data shape, commands, build steps, or release state. If it did:

- update `docs/PRODUCT_SPEC.md` when intended product behavior changed;
- update `docs/PROJECT_CONTEXT.md` when actual implementation or build state changed;
- update `docs/DECISIONS.md` when a long-lived decision was added or deliberately reversed;
- update this file only when every future agent needs a new stable rule.

If a later requirement reverses a recorded decision, update the code and decision record together and explain the reason. Never leave documentation knowingly describing an obsolete implementation.
