<p align="center">
  <img src="./src-tauri/icons/128x128.png" width="96" height="96" alt="M²Shelf Logo">
</p>

<h1 align="center">M²Shelf</h1>

<p align="center"><strong>MORI MEDIA SHELF</strong></p>

<p align="center"><a href="./README.md">简体中文</a> · <a href="./README.en-US.md">English</a> · <a href="./README.ko-KR.md">한국어</a></p>

<p align="center">すべて GPT によって制作された、Windows のローカルメディアコレクション向けのローカルファースト・メディアソース読み取り専用ブラウザーです。</p>

M²Shelf は、内蔵・外付けドライブや NAS のマッピングフォルダーに保存されたアニメ、映画、関連リソースを独立したインデックスとして管理し、ポスター表示、Bangumi メタデータ、タグ、お気に入り、視聴履歴、外部プレーヤーへの起動導線を提供します。

簡単に言えば、エクスプローラー上で言語、エンコード、字幕グループの違いや長すぎるファイル名のため判別しにくいアニメコレクションを、ワンクリックで次のような見やすいポスター表示に変換できます。

<img width="1445" height="1226" alt="M²Shelf のポスター表示" src="https://github.com/user-attachments/assets/4e1a235c-ea75-4996-b7a7-5c890e0b0803" />

アニメの「作品」詳細ページでは元のファイル名も引き続き確認でき、ワンクリックでエクスプローラー上の場所を開けます。

<img width="1445" height="1226" alt="M²Shelf のアニメ作品詳細ページ" src="https://github.com/user-attachments/assets/9bac84e4-af4e-4b2a-8c5f-ecd05b06b34b" />

アニメの「シリーズ」詳細ページ：

<img width="1445" height="1226" alt="M²Shelf のアニメシリーズ詳細ページ" src="https://github.com/user-attachments/assets/9bc04a4c-4184-4dd3-82dc-094ab32049f6" />

**元のメディアファイルを移動、削除、名前変更、編集することはなく、既存のフォルダー構成を整理し直す必要もありません。**

## 主な機能

- 複数のメディアライブラリを管理し、任意の深さまでフォルダーを再帰スキャン。
- すべてのリソース、個別ライブラリ、実際のフォルダー階層をポスターまたはリストで表示。
- ローカル名、ファイル名、Bangumi の多言語タイトル、ユーザータグを検索。
- 作品、シリーズ、その他のリソースを自動分類し、手動分類を維持。
- 高い確度の Bangumi 項目を自動マッチングし、手動検索、修正、カバー再取得にも対応。
- 動画と、字幕、画像、音声、文書、圧縮ファイルなどの関連リソースを表示。
- 設定した外部プレーヤーで再生し、Windows エクスプローラーでファイルを表示。
- タグ、1 階層の名前付きお気に入りフォルダー、編集モードによる一括整理。
- M²Shelf が再生を正常に開始した作品を新しい順で表示。
- 简体中文、English、日本語、한국어に対応。
- システム設定、ライト、ダークの各テーマに対応。
- ウィンドウサイズ、並び順、各ブラウズ領域のセッション内位置を記憶。
- アプリ所有のカバーキャッシュ保存先を変更可能。

## ローカルファーストとプライバシー

メディアフォルダーは常に読み取り専用として扱われます。インデックス、表示名、Bangumi の紐付け、タグ、お気に入り、視聴履歴、設定は M²Shelf 専用の SQLite データベースに保存され、カバー画像はアプリのキャッシュに保存されます。

Bangumi の検索とカバー取得にはインターネット接続が必要です。ローカルインデックスの閲覧とローカルファイルの起動は Bangumi に依存しません。メディアサーバーやクラウドアカウントは不要で、メディアファイルを外部サービスへアップロードすることもありません。

## ダウンロード

現在のバージョン：**M²Shelf 0.5.11**（Windows x64）

- [Portable 版をダウンロード](https://github.com/Undermori/M2Shelf/releases/download/v0.5.11/M2Shelf-Portable-0.5.11-x64.zip)
- [最新 Release を表示](https://github.com/Undermori/M2Shelf/releases/latest)
- [すべての Release を表示](https://github.com/Undermori/M2Shelf/releases)

Portable 版の使い方：

1. ZIP 全体を展開し、圧縮ファイル内から直接起動しないでください。
2. `M2Shelf.exe` をダブルクリック。
3. メディアフォルダーを追加してスキャン。
4. 必要に応じて外部プレーヤーのパスを設定。

Portable はアプリ本体のインストールが不要という意味です。データベース、設定、既定のカバーキャッシュは Windows のアプリデータフォルダーに保存されます。現在のビルドはコード署名されていないため、Windows SmartScreen が「不明な発行元」と表示する場合があります。画面表示には Microsoft Edge WebView2 Runtime が必要です。

## 現在の範囲

M²Shelf は現在、内蔵プレーヤー、オンライン動画、トランスコード、メディアサーバー、アカウント同期、自動字幕、続きから再生するための進捗管理には対応していません。メディアファイルを自動で移動または名前変更することもありません。

## 開発

技術スタック：Tauri 2、Rust、React 19、TypeScript、Vite、SQLite。

```powershell
npm install
npm run tauri dev
```

コミット前の確認：

```powershell
npm run typecheck
npm run build
npm run validate
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --locked -- -D warnings
```

Windows 向けの正式な成果物は `scripts/build_windows_release.ps1` で作成します。

## プロジェクト文書

- [開発ルール](./AGENTS.md)
- [製品仕様](./docs/PRODUCT_SPEC.md)
- [現在の実装](./docs/PROJECT_CONTEXT.md)
- [長期的な決定事項](./docs/DECISIONS.md)
- [プロジェクト早見ガイド](./PROJECT_DOCUMENTATION.md)

## 作者

- [森下Undermori · Bilibili](https://space.bilibili.com/2903441)
