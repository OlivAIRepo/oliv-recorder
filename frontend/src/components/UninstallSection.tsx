"use client"

import { useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import Analytics from "@/lib/analytics"

/**
 * Danger-zone card: uninstall the app from within Settings. Two-step inline
 * confirm; the Rust `uninstall_app` command removes the login item, clears the
 * keychain login, deletes app data + the app itself, and exits. Meeting
 * recordings are kept.
 */
export function UninstallSection() {
  const [confirming, setConfirming] = useState(false);
  const [uninstalling, setUninstalling] = useState(false);
  const [error, setError] = useState<string | null>(null);

  return (
    <div className="mt-6 bg-white rounded-xl border border-red-200 p-6">
      <h2 className="text-lg font-semibold text-gray-900">Uninstall Oliv AI</h2>
      <p className="mt-1 text-sm text-gray-500">
        Removes the app, its data, and your login from this computer.
      </p>
      {error && <p className="mt-3 text-sm text-red-600">{error}</p>}
      {!confirming ? (
        <button
          onClick={() => setConfirming(true)}
          className="mt-4 inline-flex items-center gap-2 rounded-lg border border-red-300 px-4 py-2.5 text-sm font-medium text-red-600 hover:bg-red-50 transition-colors"
        >
          Uninstall
        </button>
      ) : (
        <div className="mt-4 flex flex-wrap items-center gap-3">
          <span className="text-sm font-medium text-red-700">
            Are you sure? The app will close and remove itself.
          </span>
          <button
            onClick={async () => {
              setUninstalling(true);
              setError(null);
              try {
                await Analytics.track('app_uninstalled', {});
                await invoke('uninstall_app');
              } catch (e) {
                setUninstalling(false);
                setConfirming(false);
                setError(String(e));
              }
            }}
            disabled={uninstalling}
            className="rounded-lg bg-red-600 px-4 py-2.5 text-sm font-medium text-white hover:bg-red-700 disabled:opacity-60 transition-colors"
          >
            {uninstalling ? 'Uninstalling…' : 'Yes, uninstall'}
          </button>
          <button
            onClick={() => setConfirming(false)}
            disabled={uninstalling}
            className="rounded-lg border border-gray-300 px-4 py-2.5 text-sm text-gray-600 hover:bg-gray-100 transition-colors"
          >
            Cancel
          </button>
        </div>
      )}
    </div>
  );
}
