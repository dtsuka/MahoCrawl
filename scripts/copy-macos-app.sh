#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_root"

app_source="$project_root/src-tauri/target/release/bundle/macos/MahoCrawl.app"
release_dir="$project_root/release"

if [[ ! -d "$app_source" ]]; then
  printf 'ビルド済みのアプリが見つかりません: %s\n' "$app_source" >&2
  exit 1
fi

mkdir -p "$release_dir"
copy_dir="$(mktemp -d "$release_dir/.copy-XXXXXX")"
trap 'rm -rf "$copy_dir"' EXIT

ditto "$app_source" "$copy_dir/MahoCrawl.app"
rm -rf "$release_dir/MahoCrawl.app"
mv "$copy_dir/MahoCrawl.app" "$release_dir/MahoCrawl.app"

printf '\nアプリを出力しました: %s/MahoCrawl.app\n' "$release_dir"
