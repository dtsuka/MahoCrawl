# MahoCrawl 開発・ビルドガイド

ソースコードから MahoCrawl をビルド・開発するための手順です。アプリを利用するだけなら [README](../README.md) の手順でビルド済みアプリをダウンロードしてください。

## 構成

MahoCrawl は、SiteOne Crawler v2.5.1 を安全な Tauri v2 bridge から呼び出す macOS 向けアプリです。UI は Vue 3 + TypeScript + Vite で構成しています。

- `src/`: Vue 3 UI、設定 CRUD、ギャラリー、ログ、フロント側検証
- `src-tauri/src/lib.rs`: 設定永続化、入力検証、SiteOne の限定コマンド、sidecar process state、イベント配信、キャプチャ列挙
- `src-tauri/binaries/`: Tauri externalBin 規約に合わせた arm64 / x86_64 の SiteOne Crawler（`npm run build:siteone` で生成。Git 管理対象外）
- `patches/`: SiteOne Crawler / chromiumoxide へのカスタムパッチ
- `Resources/Licenses/`: 同梱 SiteOne Crawler のライセンスと第三者ライセンス一覧
- `scripts/`: SiteOne のビルド、ライセンス一覧の生成、リリース用ビルドなどの補助スクリプト
- `docs/design/maho-crawl-tauri-concept.png`: 実装のレイアウト・配色基準

フロントエンドから任意の shell コマンドは実行できません。sidecar の起動は Rust の `start_crawl` に限定し、引数は `Vec<String>` として渡します。停止、レポート/保存先/元画像を開く操作も Rust command の個別引数として扱います。

Rust bridge は URL、深度、workers、リクエスト速度、timeout、保存先、サイズの寸法/ID/重複を再検証します。設定は Tauri の app config directory とブラウザ fallback の localStorage に保存されます。

## 必要な環境

- macOS 13 以降
- Node.js
- Rust（rustup 版。Homebrew 版の Rust には x86_64 の標準ライブラリがないため、Intel 版のビルドに失敗します）
- [cargo-about](https://github.com/EmbarkStudios/cargo-about)（第三者ライセンス一覧の生成に使用）

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo install cargo-about --locked --features cli
```

PATH 上で Homebrew 版の `cargo` が優先される場合は、`export PATH="$HOME/.cargo/bin:$PATH"` を設定してからビルドしてください。

## セットアップ

SiteOne Crawler のバイナリはリポジトリに含まれていないため、最初にカスタムビルドを生成してください（数分かかります）。

```bash
npm install
npm run build:siteone
```

## 開発・テスト

```bash
npm run dev
npm run test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
npm run tauri dev
```

`npm run dev` はブラウザ上でサンプルデータを使った UI プレビューを起動します。実際のクロールは `npm run tauri dev` のデスクトップアプリで行います。

Tauri アプリのデバッグ bundle は次で作成します。

```bash
npm run tauri build -- --debug
```

## アプリのビルド

ローカルで使うリリースビルド（実行中の Mac のアーキテクチャ向け）は次で作成します。

```bash
npm run build:app
```

`build:app` は第三者ライセンス一覧（`Resources/Licenses/THIRD_PARTY_LICENSES.txt`）を再生成してからアプリをビルドします。Rust のビルド成果物にはローカルの絶対パスを残さないよう `--remap-path-prefix` を付けています。

ビルド成功後、`src-tauri/target/release/bundle/macos/MahoCrawl.app` をプロジェクト直下の `release/MahoCrawl.app` に自動コピーします。次回以降はコピー完了後に前回のアプリを置き換えます。`release/` は Git 管理対象外です。

`npm run build` はフロントエンドのみのビルド、`npm run tauri build` は Tauri 標準の出力先へのビルドです。

## リリース

GitHub Releases 掲載用の zip は次で作成します。Apple Silicon 版と Intel 版をそれぞれビルドし、`release/MahoCrawl_<version>_aarch64.zip` と `release/MahoCrawl_<version>_x86_64.zip` を出力します。

```bash
npm run build:release
```

アプリは ad-hoc 署名（`tauri.conf.json` の `signingIdentity: "-"`）されますが、Developer ID 署名と公証は行いません。リリース手順は次のとおりです。

1. `package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml` のバージョンを更新してコミットする
2. `npm run build:release` で zip を作成する
3. GitHub Release を作成して zip を添付する

```bash
gh release create v<version> release/MahoCrawl_<version>_aarch64.zip release/MahoCrawl_<version>_x86_64.zip --title "MahoCrawl v<version>"
```

### Developer ID 署名と公証

Developer ID で署名・公証して配布する場合は、hardened runtime を有効にして署名し、notarization を行います。

```bash
npm run build:app
(cd src-tauri/binaries && shasum -a 256 -c ../../Resources/Binaries/checksums.sha256)
codesign --force --options runtime --deep --sign "Developer ID Application: <Your Name>" release/MahoCrawl.app
xcrun notarytool submit release/MahoCrawl.app --keychain-profile "<profile>" --wait
xcrun stapler staple release/MahoCrawl.app
```

## SiteOne Crawler のカスタムビルド

MahoCrawl は SiteOne Crawler v2.5.1 ベースの非公式カスタムビルドを同梱します。変更内容は次のとおりです。

- ブラウザ撮影時に、対象 URL と完全一致する origin だけへ Basic 認証を渡す
- `--http-auth-stdin` で認証情報を標準入力から受け取る（コマンドライン引数へは載せない）

カスタムビルドは次で再生成できます。

```bash
npm run build:siteone
```

パッチは `patches/` に、ビルド元のバージョン・SHA-256 はビルド時に `Resources/Binaries/` へ出力されます（バイナリと同様に Git 管理対象外）。
ビルド時は、`SITEONE_COMMIT` と `CHROMIUMOXIDE_COMMIT` 環境変数で上流ソースのコミット SHA を指定できます。デフォルト値はスクリプト内に記録され、タグの付け替えによるサプライチェーンリスクを防ぐため、取得後に SHA が一致することを検証します。

Tauri externalBin は次のターゲット名を使用します。

- `siteone-crawler-aarch64-apple-darwin`
- `siteone-crawler-x86_64-apple-darwin`

更新後は `siteone-crawler-* --version`、`cargo test`、`npm run build`、`cargo check` を実行してください。SiteOne の CLI に存在しない独自フラグは渡しません。現在の v2.5.1 には cookie banner 専用 CLI フラグがないため、設定値は保存しつつ、UI では「現在未対応」と明示して操作を無効化しています。

カスタム SiteOne の stdin 認証は `scripts/test-siteone-auth-integration.sh` でローカル検証できます。

## 出力構造の詳細

各キャプチャサイズは書込可能な専用 `.siteone-http-cache` を引数配列で受け取り、サイズ間の cache 競合を避けます。内部 cache は隠しディレクトリとしてギャラリーの列挙対象から外し、sidecar 終了後に best-effort で削除します。

サイズ名は安全な slug に変換し、寸法を必ず suffix に含めます。実行フォルダとサイズフォルダが既存の場合は suffix を付けて上書きを避けます。

## ライセンス

第三者ライセンスは `Resources/Licenses/THIRD_PARTY_NOTICES.txt` と `Resources/Licenses/THIRD_PARTY_LICENSES.txt` を参照してください。依存関係を更新したら `npm run licenses` で一覧を再生成してください。
