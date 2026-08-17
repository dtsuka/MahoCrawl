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

Rust bridge は URL、深度、workers、リクエスト速度、timeout、保存先、サイズの寸法/ID/重複/有効件数を再検証します。設定は Tauri の app config directory とブラウザ fallback の localStorage に保存されます。

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

## SiteOne 更新

同梱バイナリは `Resources/Binaries/VERSION`、`checksums.sha256` と照合してから `src-tauri/binaries/` に配置してください。Tauri externalBin は次のターゲット名を使用します。

- `siteone-crawler-aarch64-apple-darwin`
- `siteone-crawler-x86_64-apple-darwin`

更新後は `siteone-crawler-* --version`、`cargo test`、`npm run build`、`cargo check` を実行してください。SiteOne の CLI に存在しない独自フラグは渡しません。現在の v2.5.1 には cookie banner 専用 CLI フラグがないため、設定値は保存し、UI では対応サイトのみの best-effort 設定として表示しています。

## 配布上の制約

同梱バイナリは MIT ライセンスです。Developer ID 署名、hardened runtime、notarization、アプリ/サードパーティ依存のライセンス棚卸しは配布前に別途必要です。実キャプチャには Chromium 系ブラウザが必要で、利用者が許可した場合のみ SiteOne の自動ダウンロードを有効にできます。ログやレポートには対象サイトの情報が含まれるため、許可を得たサイトだけを対象にしてください。
