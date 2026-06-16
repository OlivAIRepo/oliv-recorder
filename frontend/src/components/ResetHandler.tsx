'use client';

import React, { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';

// Confirms and performs "Reset app data & log out". Triggered from the menubar
// tray (Tauri event `request-app-reset`) or the Settings button (window event
// `oliv-open-reset`). Mounted globally so it works from anywhere.
export default function ResetHandler() {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const openModal = () => setOpen(true);
    window.addEventListener('oliv-open-reset', openModal);
    const unlisten = listen('request-app-reset', () => setOpen(true));
    return () => {
      window.removeEventListener('oliv-open-reset', openModal);
      unlisten.then((fn) => fn());
    };
  }, []);

  const doReset = async () => {
    setBusy(true);
    try {
      // Restarts the app; this call won't return on success.
      await invoke('reset_app_data');
    } catch (e) {
      console.error('reset_app_data failed', e);
      setBusy(false);
      setOpen(false);
    }
  };

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-[10000] flex items-center justify-center bg-black/40">
      <div className="w-[400px] rounded-xl bg-white p-6 shadow-2xl">
        <h2 className="text-lg font-semibold text-gray-900">Reset app data &amp; log out?</h2>
        <p className="mt-2 text-sm text-gray-600">
          This logs you out and clears local app data — onboarding, the local
          database, and the downloaded transcription/summary models. The app will
          restart and re-download the models. Your recordings are kept.
        </p>
        <div className="mt-5 flex justify-end gap-3">
          <button
            onClick={() => setOpen(false)}
            disabled={busy}
            className="rounded-lg px-4 py-2 text-sm font-medium text-gray-600 hover:bg-gray-100 transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={doReset}
            disabled={busy}
            className="rounded-lg bg-red-600 px-4 py-2 text-sm font-medium text-white hover:bg-red-700 transition-colors disabled:opacity-60"
          >
            {busy ? 'Resetting…' : 'Reset & log out'}
          </button>
        </div>
      </div>
    </div>
  );
}
