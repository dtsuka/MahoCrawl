# MahoCrawl

MahoCrawl は、SiteOne Crawler v2.5.1 を安全な Tauri v2 bridge から呼び出す macOS 向けサイトキャプチャワークスペースです。Vue 3 + TypeScript + Vite の画面で URL、UA、撮影条件、複数のキャプチャサイズを設定し、1回の実行で有効サイズを順次処理します。

## 構成

- `src/`: Vue 3 UI、設定 CRUD、ギャラリー、ログ、フロント側検証
- `src-tauri/src/lib.rs`: 設定永続化、入力検証、SiteOne の限定コマンド、sidecar process state、イベント配信、キャプチャ列挙
- `src-tauri/binaries/`: Tauri externalBin 規約に合わせた arm64 / x86_64 の SiteOne Crawler
- `Resources/Licenses/`: 同梱 SiteOne Crawler の MIT ライセンス
- `docs/design/maho-crawl-tauri-concept.png`: 実装のレイアウト・配色基準

フロントエンドから任意の shell コマンドは実行できません。sidecar の起動は Rust の `start_crawl` に限定し、引数は `Vec<String>` として渡します。停止、レポート/保存先/元画像を開く操作も Rust command の個別引数として扱います。

## 開発・テスト

```bash
npm install
npm run dev
npm run test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
npm run tauri dev
```

Tauri アプリのデバッグ bundle は次で作成します。

```bash
npm run tauri build -- --debug
```

Rust bridge は URL、深度、workers、リクエスト速度、timeout、保存先、サイズの寸法/ID/重複を再検証します。設定は Tauri の app config directory とブラウザ fallback の localStorage に保存されます。

## アプリのビルド

macOS のリリース用アプリは次で作成します。

```bash
npm run build:app
```

ビルド成功後、`src-tauri/target/release/bundle/macos/MahoCrawl.app` をプロジェクト直下の `release/MahoCrawl.app` に自動コピーします。次回以降はコピー完了後に前回のアプリを置き換えます。`release/` は Git 管理対象外です。

`npm run build` はフロントエンドのみのビルド、`npm run tauri build` は Tauri 標準の出力先へのビルドです。

## 画面の使い方

- メタ情報だけ確認する場合は、キャプチャサイズをすべてオフ、または削除してクロールを開始します。撮影せずに1回クロールし、自動的に行表示で結果を表示します。ブラウザを起動せずHTML内の情報を取得するため、JavaScriptによって後から追加される情報は対象外です。
- ギャラリーは「グリッド」と「行」を切り替えられます。行表示では1ページを1行で確認できます。
- 検索欄では URL、SEO項目、ファイル名を検索でき、解除ボタンで条件を戻せます。
- 行のキャプチャサイズ名を選ぶと、そのページ・サイズに明示的に対応する画像だけをモーダルで開きます。モーダルでは「全体」と「100%」を切り替えられ、`Esc` で閉じられます。
- SEOの「詳細」では省略されていないページ情報を確認できます。
- ブラウザで表示されるデータはサンプルです。実際のクロール実行は Tauri デスクトップアプリで行います。
- 設定は入力内容に応じて自動保存されます。

## 複数サイズの出力構造

有効サイズはデフォルトで Desktop `1440x900`、Tablet `768x1024`、Mobile `390x844` です。サイズは直列に処理され、実行ごとに次の構造を作ります。

```text
~/Pictures/MahoCrawl/example.com-20260818-120000/
├── desktop-1440x900/
│   ├── screenshots/
│   ├── .siteone-http-cache/  # 実行中だけ使う内部cache（gallery列挙対象外、終了後にbest-effort削除）
│   ├── report.html
│   ├── report.json
│   └── report.txt
├── tablet-768x1024/
│   └── ...
└── mobile-390x844/
    └── ...
```

サイズ名は安全な slug に変換し、寸法を必ず suffix に含めます。実行フォルダとサイズフォルダが既存の場合は suffix を付けて上書きを避けます。
各サイズは書込可能な専用 `.siteone-http-cache` を引数配列で受け取り、サイズ間のcache競合を避けます。内部cacheは隠しディレクトリとして列挙せず、sidecar終了後にbest-effortで削除します。

キャプチャサイズがすべて無効、または0件の場合は、実行フォルダ内の `metadata/` に `report.html`、`report.json`、`report.txt` を出力します。スクリーンショットは生成しません。

## SiteOne 更新

MahoCrawl は SiteOne Crawler v2.5.1 ベースの非公式カスタムビルドを同梱します。変更内容は次のとおりです。

- ブラウザ撮影時に、対象 URL と完全一致する origin だけへ Basic 認証を渡す
- `--http-auth-stdin` で認証情報を標準入力から受け取る（コマンドライン引数へは載せない）

カスタムビルドは次で再生成できます。

```bash
npm run build:siteone
```

ビルド元の commit、パッチ、SHA-256 は `patches/` と `Resources/Binaries/` に記録されます。Tauri externalBin は次のターゲット名を使用します。

- `siteone-crawler-aarch64-apple-darwin`
- `siteone-crawler-x86_64-apple-darwin`

Intel 向けビルドには `rustup target add x86_64-apple-darwin` が必要です。更新後は `siteone-crawler-* --version`、`cargo test`、`npm run build`、`cargo check` を実行してください。SiteOne の CLI に存在しない独自フラグは渡しません。現在の v2.5.1 には cookie banner 専用 CLI フラグがないため、設定値は保存しつつ、UIでは「現在未対応」と明示して操作を無効化しています。

Basic 認証のパスワードは `configuration.json` とブラウザの localStorage へ保存しません。アプリ再起動後は再入力が必要です。第三者ライセンスは `Resources/Licenses/THIRD_PARTY_NOTICES.txt` を参照してください。

## 配布上の制約

同梱バイナリは MIT ライセンスです。Developer ID 署名、hardened runtime、notarization、アプリ/サードパーティ依存のライセンス棚卸しは配布前に別途必要です。実キャプチャには Chromium 系ブラウザが必要で、利用者が許可した場合のみ SiteOne の自動ダウンロードを有効にできます。ログやレポートには対象サイトの情報が含まれるため、許可を得たサイトだけを対象にしてください。

配布前の署名・公証は次の流れです。

```bash
npm run build:app
shasum -a 256 -c Resources/Binaries/checksums.sha256
codesign --force --options runtime --deep --sign "Developer ID Application: <Your Name>" release/MahoCrawl.app
xcrun notarytool submit release/MahoCrawl.app --keychain-profile "<profile>" --wait
xcrun stapler staple release/MahoCrawl.app
```

カスタム SiteOne の stdin 認証は `scripts/test-siteone-auth-integration.sh` でローカル検証できます。
