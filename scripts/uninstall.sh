#!/usr/bin/env bash
#
# Oliv AI — complete uninstall / reset (macOS).
#
# Removes EVERYTHING: the app, onboarding state, downloaded models, local DB,
# the logged-in token (keychain), and the microphone + screen-recording
# permission grants. Use it to fully uninstall, or to test the brand-new-user
# experience from a clean slate.
#
#   bash uninstall.sh                 # removes app + data + login + permissions
#   KEEP_RECORDINGS=0 bash uninstall.sh   # ALSO delete recordings (~/Movies/oliv)
#
# Recordings are KEPT by default (they're your data); set KEEP_RECORDINGS=0 to wipe them.

set -uo pipefail

BUNDLE_ID="ai.oliv.recorder"
APP="/Applications/Oliv AI.app"
OLD_APP="/Applications/Oliv Recorder.app"   # pre-rename name
KEEP_RECORDINGS="${KEEP_RECORDINGS:-1}"

echo "Uninstalling Oliv AI…"

# 1. Quit the app if running.
osascript -e 'quit app "Oliv AI"' 2>/dev/null || true
pkill -f "$APP" 2>/dev/null || true
pkill -f "$OLD_APP" 2>/dev/null || true
sleep 1

# 2. Remove the app bundle(s).
rm -rf "$APP" "$OLD_APP"

# 3. App data, caches, web state, preferences (everything keyed by bundle id).
rm -rf "$HOME/Library/Application Support/$BUNDLE_ID"
rm -rf "$HOME/Library/Caches/$BUNDLE_ID"
rm -rf "$HOME/Library/WebKit/$BUNDLE_ID"
rm -rf "$HOME/Library/HTTPStorages/$BUNDLE_ID" "$HOME/Library/HTTPStorages/$BUNDLE_ID.binarycookies"
rm -rf "$HOME/Library/Saved Application State/$BUNDLE_ID.savedState"
rm -f  "$HOME/Library/Preferences/$BUNDLE_ID.plist"

# 4. Logged-in token (macOS keychain).
security delete-generic-password -s "$BUNDLE_ID" >/dev/null 2>&1 || true

# 5. Permission grants (microphone + screen recording / system audio).
tccutil reset Microphone "$BUNDLE_ID" 2>/dev/null || true
tccutil reset ScreenCapture "$BUNDLE_ID" 2>/dev/null || true

# 6. Recordings (kept by default).
if [ "$KEEP_RECORDINGS" = "0" ]; then
  rm -rf "$HOME/Movies/oliv"
  echo "  • Removed recordings (~/Movies/oliv)"
else
  echo "  • Kept recordings (~/Movies/oliv) — rerun with KEEP_RECORDINGS=0 to delete"
fi

echo "Done — Oliv AI fully removed (app, data, login, permissions)."
