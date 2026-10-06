#!/usr/bin/env bash
# Acceptance A01 (automated part): install the freshly built package the way a
# rider would, launch the installed app with RIDGELINE_SMOKE_TEST set, and
# require a report proving that the bundled UI loaded, reached the native
# service over IPC, opened demo mode and saw simulated devices become ready.
# The app uses a throwaway data folder in this mode.
#
# Failures are reported as GitHub annotations (including the failing command
# and the tail of the app's output) so they're visible without downloading logs.
#
# usage: scripts/smoke-installed.sh <bundle_dir> <report.json>
set -uo pipefail
BUNDLE="$1"
REPORT="$(pwd)/$2"
LOG="$(pwd)/smoke-app.log"
rm -f "$REPORT" "$LOG"
WAIT=150

fail() {
  echo "::error::A01 smoke test: $*"
  if [ -s "$LOG" ]; then
    tail -n 25 "$LOG" | sed 's/^/::error::app output: /'
  fi
  exit 1
}

# First match of a pattern without tripping pipefail on SIGPIPE.
first() { awk 'NR==1 {print; exit}'; }

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
    DEB=$(ls "$BUNDLE"/deb/*.deb 2>/dev/null | first)
    [ -n "$DEB" ] || fail "no .deb in $BUNDLE/deb"
    echo "Installing $DEB"
    sudo apt-get install -y xvfb > /dev/null || fail "could not install xvfb"
    sudo apt-get install -y "$(realpath "$DEB")" || fail "apt could not install $DEB"
    PKG=$(dpkg-deb -f "$DEB" Package)
    BIN=$(dpkg -L "$PKG" | grep -E '^/usr/bin/[^/]+$' | first)
    [ -n "$BIN" ] && [ -x "$BIN" ] || fail "installed binary not found for package '$PKG' (files: $(dpkg -L "$PKG" | grep bin | tr '\n' ' '))"
    echo "Launching $BIN"
    export WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1 LIBGL_ALWAYS_SOFTWARE=1
    RIDGELINE_SMOKE_TEST="$REPORT" xvfb-run -a -s "-screen 0 1440x900x24" "$BIN" > "$LOG" 2>&1 &
    wait_report $!
    ;;
  Darwin)
    DMG=$(ls "$BUNDLE"/dmg/*.dmg 2>/dev/null | first)
    [ -n "$DMG" ] || fail "no .dmg in $BUNDLE/dmg"
    echo "Mounting $DMG"
    hdiutil verify "$DMG" > /dev/null 2>&1 && echo "Disk image checksum verified" || echo "::warning::hdiutil verify did not succeed for $DMG"
    MNT=$(mktemp -d)
    MOUNTED=""
    for attempt in 1 2 3 4 5; do
      if [ "$attempt" -le 2 ]; then
        # The image carries the MIT license as an agreement; accept it non-interactively.
        if OUT=$(printf 'Y\nY\n' | PAGER=cat hdiutil attach -nobrowse -readonly -noautoopen -noverify -mountpoint "$MNT" "$DMG" 2>&1); then MOUNTED=1; break; fi
      else
        # Fall back to the default /Volumes mount point.
        if OUT=$(printf 'Y\nY\n' | PAGER=cat hdiutil attach -nobrowse -readonly -noautoopen -noverify "$DMG" 2>&1); then
          MNT=$(printf '%s\n' "$OUT" | grep -o '/Volumes/.*' | tail -n 1)
          [ -n "$MNT" ] && MOUNTED=1 && break
        fi
      fi
      echo "hdiutil attach attempt $attempt failed: $(printf '%s' "$OUT" | tail -n 3 | tr '\n' ' ')"
      sleep 5
    done
    [ -n "$MOUNTED" ] || fail "hdiutil could not mount $DMG: $(printf '%s' "$OUT" | tail -n 3 | tr '\n' ' ')"
    DEST=$(mktemp -d)
    APPSRC=$(ls -d "$MNT"/*.app 2>/dev/null | first)
    [ -n "$APPSRC" ] || fail "no .app inside the disk image"
    ditto "$APPSRC" "$DEST/$(basename "$APPSRC")" || fail "could not copy the app out of the disk image"
    hdiutil detach "$MNT" > /dev/null 2>&1 || hdiutil detach -force "$MNT" > /dev/null 2>&1 || true
    APP="$DEST/$(basename "$APPSRC")"
    EXE=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$APP/Contents/Info.plist") || fail "no CFBundleExecutable"
    echo "Launching $APP ($EXE); architectures: $(lipo -archs "$APP/Contents/MacOS/$EXE" 2>&1)"
    /usr/libexec/PlistBuddy -c 'Print :NSBluetoothAlwaysUsageDescription' "$APP/Contents/Info.plist" > /dev/null || fail "NSBluetoothAlwaysUsageDescription missing from Info.plist"
    RIDGELINE_SMOKE_TEST="$REPORT" "$APP/Contents/MacOS/$EXE" > "$LOG" 2>&1 &
    wait_report $!
    ;;
  MINGW*|MSYS*|CYGWIN*)
    SETUP=$(ls "$BUNDLE"/nsis/*-setup.exe 2>/dev/null | first)
    [ -n "$SETUP" ] || fail "no NSIS setup in $BUNDLE/nsis"
    echo "Installing $SETUP (silent, current user)"
    MSYS_NO_PATHCONV=1 "$SETUP" /S || fail "installer returned an error"
    APP=""
    for _ in $(seq 1 60); do
      APP=$(find "$(cygpath -u "$LOCALAPPDATA")" -maxdepth 3 -iname 'ridgeline.exe' 2>/dev/null | first)
      [ -n "$APP" ] && break
      sleep 1
    done
    [ -n "$APP" ] || fail "installed Ridgeline.exe not found under LOCALAPPDATA"
    echo "Launching $APP"
    RIDGELINE_SMOKE_TEST="$(cygpath -m "$REPORT")" "$APP" > "$LOG" 2>&1 &
    wait_report $!
    ;;
  *)
    fail "unsupported OS $(uname -s)" ;;
esac

[ -s "$REPORT" ] || fail "no report written (the app did not load its UI, could not reach the native service, or crashed)"
cat "$REPORT"
REPORT_NODE="$REPORT"
if command -v cygpath > /dev/null 2>&1; then REPORT_NODE=$(cygpath -m "$REPORT"); fi
node -e 'const r=JSON.parse(require("fs").readFileSync(process.argv[1],"utf8")); if(!r.ok){console.log("::error::A01 smoke test: report not ok: "+JSON.stringify(r)); process.exit(1)}' "$REPORT_NODE" || exit 1
node -e 'const r=JSON.parse(require("fs").readFileSync(process.argv[1],"utf8")); console.log("::warning title=A01 smoke report (informational)::passed "+JSON.stringify(r))' "$REPORT_NODE"
echo "Smoke test passed."
