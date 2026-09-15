'use client';

import { useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { useRouter, usePathname } from 'next/navigation';
import { useRecordingState } from '@/contexts/RecordingStateContext';

interface StartPayload {
  app: string;
  /** Stable id of the app: macOS bundle id, Windows exe / package family name. */
  source?: string;
  sensitive: boolean;
}

// Headless. The visible "meeting detected" prompt is a separate floating window
// (public/meeting-prompt.html) shown at the top-right of the screen by the Rust
// mic monitor. When the user (or its countdown) chooses to record, that window
// emits `start-recording-from-prompt`; here in the main window we run the same
// start path the tray/sidebar use.
export default function MeetingDetectedPrompt() {
  const router = useRouter();
  const pathname = usePathname();
  const { isRecording } = useRecordingState();

  // Refs so the (mount-once) listener reads live values.
  const isRecordingRef = useRef(isRecording);
  const pathnameRef = useRef(pathname);
  useEffect(() => {
    isRecordingRef.current = isRecording;
  }, [isRecording]);
  useEffect(() => {
    pathnameRef.current = pathname;
  }, [pathname]);

  useEffect(() => {
    const unlisten = listen<StartPayload>('start-recording-from-prompt', async (event) => {
      if (isRecordingRef.current) return;
      const { app, source, sensitive } = event.payload;
      // Rust holds the authoritative flag and rebroadcasts `sensitive-changed`,
      // which the Home screen's checkbox follows.
      await invoke('oliv_set_sensitive', { sensitive }).catch(() => {});
      // Tag the source app for the ingest session, but don't use it as the
      // meeting name — names are always auto-generated (Meeting dd_mm_yy_…).
      // `appId` is the stable id the backend routes on (a WhatsApp call is
      // logged as a call, a meeting app as a meeting).
      await invoke('oliv_set_source_app', { app, appId: source ?? null }).catch(() => {});
      if (pathnameRef.current === '/') {
        window.dispatchEvent(new CustomEvent('start-recording-from-sidebar'));
      } else {
        try {
          sessionStorage.setItem('autoStartRecording', 'true');
        } catch {
          /* sessionStorage unavailable */
        }
        router.push('/');
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [router]);

  return null;
}
