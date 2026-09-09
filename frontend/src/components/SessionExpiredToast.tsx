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

export default function SessionExpiredToast() {
  useEffect(() => {
    const unlistenLost = listen('oliv-auth-lost', () => {
      toast.error('Oliv session expired', {
        id: TOAST_ID, // stable id: repeated events update one toast, never stack
        description:
          'Your meetings are recording locally but not syncing to Oliv. Reconnect to resume uploads.',
        duration: Infinity,
        action: {
          label: 'Reconnect',
          onClick: () => {
            invoke('open_external_url', { url: OLIV_LOGIN_URL }).catch((e) =>
              console.error('Failed to open Oliv login:', e),
            );
          },
        },
      });
    });

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
