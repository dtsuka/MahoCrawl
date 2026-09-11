#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
patch_dir="$project_root/patches"
build_root="${SITEONE_BUILD_ROOT:-$project_root/.siteone-build}"
bin_dir="$project_root/src-tauri/binaries"
metadata_dir="$project_root/Resources/Binaries"

SITEONE_REPO="${SITEONE_REPO:-https://github.com/janreges/siteone-crawler.git}"
SITEONE_REF="${SITEONE_REF:-v2.5.1}"
CHROMIUMOXIDE_REPO="${CHROMIUMOXIDE_REPO:-https://github.com/mattsse/chromiumoxide.git}"
CHROMIUMOXIDE_REF="${CHROMIUMOXIDE_REF:-v0.9.1}"

targets=(
  "aarch64-apple-darwin"
  "x86_64-apple-darwin"
)

clone_or_update() {
  local dir="$1"
  local repo="$2"
  local ref="$3"
  if [[ ! -d "$dir/.git" ]]; then
    git clone --depth 1 --branch "$ref" "$repo" "$dir"
  else
    git -C "$dir" fetch --depth 1 origin "$ref"
    git -C "$dir" checkout -f FETCH_HEAD
  fi
}

apply_patch() {
  local repo_dir="$1"
  local patch_file="$2"
  git -C "$repo_dir" reset --hard
  git -C "$repo_dir" clean -fdx
  git -C "$repo_dir" apply --whitespace=nowarn "$patch_file"
}

mkdir -p "$build_root" "$bin_dir" "$metadata_dir"

chromiumoxide_dir="$build_root/chromiumoxide"
siteone_dir="$build_root/siteone-crawler"

clone_or_update "$chromiumoxide_dir" "$CHROMIUMOXIDE_REPO" "$CHROMIUMOXIDE_REF"
clone_or_update "$siteone_dir" "$SITEONE_REPO" "$SITEONE_REF"

apply_patch "$chromiumoxide_dir" "$patch_dir/chromiumoxide-v0.9.1-scoped-auth.patch"
apply_patch "$siteone_dir" "$patch_dir/siteone-crawler-v2.5.1-browser-auth.patch"

if ! grep -q '\[patch.crates-io\]' "$siteone_dir/Cargo.toml"; then
  cat >>"$siteone_dir/Cargo.toml" <<'EOF'

[patch.crates-io]
chromiumoxide = { path = "../chromiumoxide" }
EOF
fi

checksum_file="$metadata_dir/checksums.sha256"
: >"$checksum_file"

host_target="$(rustc -vV | awk '/^host: / { print $2 }')"
lock_args=()
if [[ -f "$siteone_dir/Cargo.lock" ]]; then
  lock_args=(--locked)
fi

for target in "${targets[@]}"; do
  build_target=("$target")
  if ! rustup target list --installed 2>/dev/null | grep -qx "$target"; then
    if command -v rustup >/dev/null 2>&1; then
      rustup target add "$target"
    elif [[ "$host_target" == "$target" ]]; then
      printf '注意: rustup 未使用のためホスト target (%s) をネイティブビルドします。\n' "$target" >&2
      build_target=()
    else
      printf '警告: %s 向けの Rust target が未インストールです。rustup target add %s を実行してください。\n' "$target" "$target" >&2
      continue
    fi
  fi

  printf 'ビルド中: %s\n' "$target"
  cargo build --manifest-path "$siteone_dir/Cargo.toml" --release "${lock_args[@]}" "${build_target[@]/#/--target }"

  if ((${#build_target[@]})); then
    output="$siteone_dir/target/$target/release/siteone-crawler"
  else
    output="$siteone_dir/target/release/siteone-crawler"
  fi
  dest="$bin_dir/siteone-crawler-$target"
  install -m 755 "$output" "$dest"
  shasum -a 256 "$dest" | awk '{print $1 "  siteone-crawler-'$target'"}' >>"$checksum_file"
done

cat >"$metadata_dir/BUILD_METADATA.json" <<EOF
{
  "siteoneRef": "$SITEONE_REF",
  "chromiumoxideRef": "$CHROMIUMOXIDE_REF",
  "rustc": "$(rustc --version)",
  "builtAt": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "customization": "MahoCrawl browser Basic auth (scoped CDP + --http-auth-stdin)"
}
EOF

printf '\nカスタム SiteOne バイナリを更新しました: %s\n' "$bin_dir"
