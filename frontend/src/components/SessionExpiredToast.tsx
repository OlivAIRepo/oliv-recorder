'use client';

import { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { OLIV_LOGIN_URL } from '@/lib/olivAuth';

// A persistent, non-blocking "reconnect" prompt shown when the middleware
// rejects our ic_token (401/403 — the token was evicted/invalidated server-side,
// see auth.rs::notify_auth_lost). Recording keeps running locally; this only
// tells the user their meetings aren't syncing until they reconnect.
//
// Deliberately NOT a login-gate flip: the rejection can first land mid-meeting,
// and hiding the recording UI then would be worse than a toast. On reconnect the
// deep-link handler emits `oliv-auth-changed`, which dismisses this.
const TOAST_ID = 'oliv-auth-lost';

function showReconnectToast() {
  toast.error('Oliv session expired', {
    id: TOAST_ID, // stable id: repeated events update one toast, never stack
    description:
      'Your meetings are recording locally but not syncing to Oliv. Reconnect to resume uploads.',
    duration: Infinity,
    // Sticky: no auto-timeout, no close button, no swipe-to-dismiss. It clears
    // only on reconnect (oliv-auth-changed) so a signed-out user can't miss it.
    dismissible: false,
    closeButton: false,
    action: {
      label: 'Reconnect',
      onClick: () => {
        invoke('open_external_url', { url: OLIV_LOGIN_URL }).catch((e) =>
          console.error('Failed to open Oliv login:', e),
        );
      },
    },
  });
}

export default function SessionExpiredToast() {
  useEffect(() => {
    // The first rejection can fire ~1s after launch, before this listener is
    // registered (Tauri doesn't buffer events for late listeners) — the common
    // "token already dead at launch" case. So also check the backend flag on
    // mount, not just the live event.
    invoke<boolean>('recorder_auth_lost')
      .then((lost) => {
        if (lost) showReconnectToast();
      })
      .catch(() => {});

    const unlistenLost = listen('oliv-auth-lost', () => showReconnectToast());

    // Successful (re-)login emits this; clear the prompt.
    const unlistenChanged = listen('oliv-auth-changed', () => {
      toast.dismiss(TOAST_ID);
    });

    return () => {
      unlistenLost.then((fn) => fn());
      unlistenChanged.then((fn) => fn());
    };
  }, []);

  return null;
}
