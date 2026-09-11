#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
binary="${SITEONE_BINARY:-$project_root/src-tauri/binaries/siteone-crawler-aarch64-apple-darwin}"

if [[ ! -x "$binary" ]]; then
  echo "SiteOne binary not found: $binary" >&2
  exit 1
fi

run_case() {
  local port="$1"
  local mode="$2"
  python3 - <<PY &
import base64
from http.server import BaseHTTPRequestHandler, HTTPServer
port = ${port}
class Handler(BaseHTTPRequestHandler):
    def do_AUTHHEAD(self):
        self.send_response(401)
        self.send_header('WWW-Authenticate', 'Basic realm="test"')
        self.end_headers()
    def do_GET(self):
        auth = self.headers.get('Authorization')
        if auth != 'Basic ' + base64.b64encode(b'user:pass').decode():
            return self.do_AUTHHEAD()
        self.send_response(200)
        self.send_header('Content-Type', 'text/html')
        self.end_headers()
        self.wfile.write(b'<html><body><h1>Authenticated</h1></body></html>')
HTTPServer(('127.0.0.1', port), Handler).serve_forever()
PY
  local server_pid=$!
  sleep 1
  local out
  out="$(mktemp -d)"
  local args=(
    "--url=http://127.0.0.1:${port}/"
    "--single-page"
    "--output-json-file=${out}/report.json"
    "--http-cache-dir=${out}/cache"
    "--http-auth-stdin"
    "--hide-progress-bar"
    "--no-color"
  )
  if [[ "$mode" == "browser" ]]; then
    args+=(
      "--browser"
      "--screenshots"
      "--screenshots-dir=${out}/shots"
    )
  fi
  printf 'user:pass' | "$binary" "${args[@]}"
  kill "$server_pid"
  grep -q 'Authenticated' "${out}/report.json"
  if [[ "$mode" == "browser" ]]; then
    test -n "$(ls -A "${out}/shots" 2>/dev/null || true)"
  fi
  echo "${mode} auth integration ok"
}

run_case 8771 http
run_case 8772 browser
