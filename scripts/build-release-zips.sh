#!/usr/bin/env bash
# GitHub Releases 掲載用に、Apple Silicon (aarch64) 版と Intel (x86_64) 版の
# MahoCrawl.app をそれぞれビルドし、release/ に zip で出力する。
# 前提: npm run build:siteone で両アーキテクチャの SiteOne Crawler を生成済み。
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_root"

# Homebrew 版 Rust には x86_64 の標準ライブラリがないため、rustup 版を優先する
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"

version="$(node -p "require('./package.json').version")"
release_dir="$project_root/release"
mkdir -p "$release_dir"

# target トリプル:配布ファイル名のアーキテクチャ表記
targets=(
  "aarch64-apple-darwin:aarch64"
  "x86_64-apple-darwin:x86_64"
)

for entry in "${targets[@]}"; do
  target="${entry%%:*}"
  arch_label="${entry##*:}"

  if [[ ! -f "src-tauri/binaries/siteone-crawler-$target" ]]; then
    printf 'SiteOne Crawler (%s) が見つかりません。先に npm run build:siteone を実行してください。\n' "$target" >&2
    exit 1
  fi

  printf '\nビルド中: %s\n' "$target"
  bash scripts/with-remapped-paths.sh npx tauri build --target "$target" --bundles app

  app_path="src-tauri/target/$target/release/bundle/macos/MahoCrawl.app"
  if [[ ! -d "$app_path" ]]; then
    printf 'ビルド済みのアプリが見つかりません: %s\n' "$app_path" >&2
    exit 1
  fi

  zip_path="$release_dir/MahoCrawl_${version}_${arch_label}.zip"
  rm -f "$zip_path"
  # ditto はリソースフォークや署名を保持したまま zip 化できる
  ditto -c -k --sequesterRsrc --keepParent "$app_path" "$zip_path"
  printf '出力しました: %s\n' "$zip_path"
done
