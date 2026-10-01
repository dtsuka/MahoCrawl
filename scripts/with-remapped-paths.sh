#!/usr/bin/env bash
# 引数のコマンドを、Rust のビルド成果物からローカルの絶対パス（ホームディレクトリ名など）を
# 取り除く --remap-path-prefix 付きで実行する。
# 例: bash scripts/with-remapped-paths.sh cargo build --release
set -euo pipefail

if (($# == 0)); then
  printf '使い方: %s <command> [args...]\n' "$0" >&2
  exit 1
fi

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
home_dir="$HOME"

# Windows の Git Bash では /c/Users/... 形式になるため、rustc が扱う C:\Users\... 形式に変換する
if command -v cygpath >/dev/null 2>&1; then
  home_dir="$(cygpath -w "$home_dir")"
  cargo_home="$(cygpath -w "$cargo_home")"
  project_root="$(cygpath -w "$project_root")"
fi

# rustc は後に指定した一致ルールを優先するため、広い範囲（ホーム）から順に並べる。
remap_flags=(
  "--remap-path-prefix=$home_dir=/home"
  "--remap-path-prefix=$cargo_home=/cargo"
  "--remap-path-prefix=$project_root=/mahocrawl"
)

export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }${remap_flags[*]}"
exec "$@"
