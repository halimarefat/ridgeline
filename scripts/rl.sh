#!/usr/bin/env bash
# Ridgeline task runner (macOS / Linux shell). Windows: scripts\rl.ps1
#
#   scripts/rl.sh setup     install UI dependencies (npm ci, or npm install without a lockfile)
#   scripts/rl.sh test      Rust workspace tests + UI typecheck and unit tests
#   scripts/rl.sh demo      build the UI and serve it at http://localhost:1420 with simulated devices
#   scripts/rl.sh dev       like demo, rebuilding the UI on change
#   scripts/rl.sh e2e       end-to-end browser walkthrough against the developer server
#   scripts/rl.sh app       run the desktop app in development mode (Tauri)
#   scripts/rl.sh package   build unsigned installers for this platform
#   scripts/rl.sh fit       write a synthetic FIT file to target/fit (for checking in other tools)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
UI="$ROOT/apps/desktop/ui"
cmd="${1:-help}"

ui_install() {
  cd "$UI"
  if [ -f package-lock.json ]; then npm ci --no-audit --no-fund; else npm install --no-audit --no-fund; fi
}
ui_build() { (cd "$UI" && [ -d node_modules ] || ui_install; cd "$UI" && npm run build); }

case "$cmd" in
  setup)
    ui_install
    ;;
  test)
    (cd "$ROOT" && cargo test --workspace)
    (cd "$UI" && [ -d node_modules ] || ui_install)
    (cd "$UI" && npm run typecheck && npm test)
    ;;
  demo)
    ui_build
    echo "Open http://localhost:1420 (data in $ROOT/.ridgeline-data)"
    cd "$ROOT" && cargo run --release -p rl-devserver -- --port 1420 --data .ridgeline-data --ui apps/desktop/ui/dist
    ;;
  dev)
    ui_build
    (cd "$UI" && npm run watch) &
    WATCH=$!
    trap 'kill $WATCH 2>/dev/null' EXIT
    cd "$ROOT" && cargo run -p rl-devserver -- --port 1420 --data .ridgeline-data --ui apps/desktop/ui/dist
    ;;
  e2e)
    ui_build
    (cd "$ROOT" && cargo build --release -p rl-devserver)
    DATA="$(mktemp -d)"
    "$ROOT/target/release/rl-devserver" --port 1421 --data "$DATA" --ui "$UI/dist" > "$DATA/server.log" 2>&1 &
    SRV=$!
    trap 'kill $SRV 2>/dev/null' EXIT
    sleep 1
    cd "$UI"
    [ -d node_modules/playwright ] || npm install --no-save --no-audit --no-fund playwright@1.56.0
    node e2e/demo-flow.mjs http://localhost:1421
    ;;
  app)
    ui_build
    cd "$ROOT/apps/desktop" && ./ui/node_modules/.bin/tauri dev
    ;;
  package)
    ui_build
    cd "$ROOT/apps/desktop"
    if [ "$(uname -s)" = "Darwin" ]; then
      rustup target add aarch64-apple-darwin x86_64-apple-darwin
      ./ui/node_modules/.bin/tauri build --target universal-apple-darwin --bundles app,dmg
    else
      ./ui/node_modules/.bin/tauri build --bundles deb,appimage
    fi
    ;;
  fit)
    cd "$ROOT" && cargo run -q -p rl-storage --example fit_fixture -- target/fit && ls -l target/fit
    ;;
  *)
    sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'
    ;;
esac
