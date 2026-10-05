#!/usr/bin/env bash
# Acceptance A01 (automated part): install the freshly built package the way a
# rider would, launch the installed app with RIDGELINE_SMOKE_TEST set, and
# require a report proving that the bundled UI loaded, reached the native
# service over IPC, opened demo mode and saw simulated devices become ready.
# The app uses a throwaway data folder in this mode.
#
# usage: scripts/smoke-installed.sh <bundle_dir> <report.json>
set -euo pipefail
BUNDLE="$1"
REPORT="$2"
rm -f "$REPORT"
WAIT=150

wait_report() {
  local pid="$1"
  for _ in $(seq 1 "$WAIT"); do
    [ -s "$REPORT" ] && break
    if ! kill -0 "$pid" 2>/dev/null; then echo "::warning::app process exited before writing a report"; break; fi
    sleep 1
  done
  sleep 2
  kill "$pid" 2>/dev/null || true
}

case "$(uname -s)" in
  Linux)
    DEB=$(ls "$BUNDLE"/deb/*.deb | head -1)
    echo "Installing $DEB"
    sudo apt-get install -y xvfb >/dev/null
    sudo apt-get install -y "$(realpath "$DEB")"
    BIN=$(dpkg-deb -c "$DEB" | awk '{print $6}' | grep '^\./usr/bin/' | head -1 | sed 's|^\.||')
    echo "Launching $BIN"
    export WEBKIT_DISABLE_COMPOSITING_MODE=1 LIBGL_ALWAYS_SOFTWARE=1
    RIDGELINE_SMOKE_TEST="$REPORT" xvfb-run -a "$BIN" > smoke-app.log 2>&1 &
    wait_report $!
    ;;
  Darwin)
    DMG=$(ls "$BUNDLE"/dmg/*.dmg | head -1)
    echo "Mounting $DMG"
    MNT=$(mktemp -d)
    hdiutil attach -nobrowse -readonly -mountpoint "$MNT" "$DMG" >/dev/null
    DEST=$(mktemp -d)
    cp -R "$MNT"/*.app "$DEST/"
    hdiutil detach "$MNT" >/dev/null || true
    APP=$(ls -d "$DEST"/*.app | head -1)
    EXE=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$APP/Contents/Info.plist")
    echo "Launching $APP ($EXE); architectures: $(lipo -archs "$APP/Contents/MacOS/$EXE")"
    RIDGELINE_SMOKE_TEST="$REPORT" "$APP/Contents/MacOS/$EXE" > smoke-app.log 2>&1 &
    wait_report $!
    ;;
  MINGW*|MSYS*|CYGWIN*)
    SETUP=$(ls "$BUNDLE"/nsis/*-setup.exe | head -1)
    echo "Installing $SETUP (silent, current user)"
    MSYS_NO_PATHCONV=1 "$SETUP" /S
    APP=""
    for _ in $(seq 1 60); do
      APP=$(find "$(cygpath -u "$LOCALAPPDATA")" -maxdepth 3 -iname 'ridgeline.exe' 2>/dev/null | head -1 || true)
      [ -n "$APP" ] && break
      sleep 1
    done
    [ -n "$APP" ] || { echo "::error::installed Ridgeline.exe not found under LOCALAPPDATA"; exit 1; }
    echo "Launching $APP"
    REPORT_W=$(cygpath -m "$REPORT")
    RIDGELINE_SMOKE_TEST="$REPORT_W" "$APP" > smoke-app.log 2>&1 &
    wait_report $!
    ;;
  *)
    echo "unsupported OS"; exit 1 ;;
esac

if [ ! -s "$REPORT" ]; then
  echo "::error::Smoke test produced no report (the app did not load its UI or crashed)."
  cat smoke-app.log 2>/dev/null | tail -40 || true
  exit 1
fi
cat "$REPORT"
REPORT_NODE="$REPORT"
command -v cygpath >/dev/null 2>&1 && REPORT_NODE=$(cygpath -m "$REPORT")
node -e 'const r=JSON.parse(require("fs").readFileSync(process.argv[1],"utf8")); if(!r.ok){console.log("::error::Smoke test failed: "+JSON.stringify(r)); process.exit(1)}' "$REPORT_NODE"
echo "Smoke test passed."
