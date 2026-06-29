#!/usr/bin/env bash
#
# Oliv AI — reset LOCAL DATA only (macOS), for repeated first-run testing.
#
# Unlike uninstall.sh, this KEEPS the installed app and the microphone /
# screen-recording permission grants — it only wipes the data that makes the
# app think it's been used before: downloaded model, local DB, login token,
# onboarding + preferences, caches, web state. Next launch behaves like a
# brand-new install, minus the reinstall and the permission prompts.
#
#   bash reset-local-data.sh              # full data reset (re-downloads model next run)
#   KEEP_MODEL=1 bash reset-local-data.sh # keep the downloaded model (skip the ~640MB re-download)
#
# Run as the logged-in user (keychain access is per-user).

set -uo pipefail

BUNDLE_ID="ai.oliv.recorder"
SUPPORT="$HOME/Library/Application Support/$BUNDLE_ID"
KEEP_MODEL="${KEEP_MODEL:-0}"

echo "Resetting Oliv AI local data…"

# 1. Quit the app so it doesn't recreate files mid-wipe.
osascript -e 'quit app "Oliv AI"' 2>/dev/null || true
pkill -f "/Applications/Oliv AI.app" 2>/dev/null || true
sleep 1

# 2. App-data dir. Either wipe it all, or keep just the downloaded model.
if [ "$KEEP_MODEL" = "1" ] && [ -d "$SUPPORT" ]; then
  find "$SUPPORT" -mindepth 1 -maxdepth 1 ! -name models -exec rm -rf {} +
  echo "  • Cleared app data, kept models/ (KEEP_MODEL=1)"
else
  rm -rf "$SUPPORT"
  echo "  • Removed app data + downloaded model"
fi

# 3. Logs, caches, web state, preferences (everything else keyed by bundle id).
rm -rf "$HOME/Library/Logs/$BUNDLE_ID"
rm -rf "$HOME/Library/Caches/$BUNDLE_ID"
rm -rf "$HOME/Library/WebKit/$BUNDLE_ID"
rm -rf "$HOME/Library/HTTPStorages/$BUNDLE_ID" "$HOME/Library/HTTPStorages/$BUNDLE_ID.binarycookies"
rm -rf "$HOME/Library/Saved Application State/$BUNDLE_ID.savedState"
rm -f  "$HOME/Library/Preferences/$BUNDLE_ID.plist"

# 4. Logged-in token(s) in the keychain. Delete every matching item (there can
#    be more than one account); each delete may pop a confirm dialog.
while security delete-generic-password -s "$BUNDLE_ID" >/dev/null 2>&1; do :; done
echo "  • Cleared login token(s) from keychain (approve any prompts)"

echo "Done — next launch is a clean first-run (app + permissions kept)."
