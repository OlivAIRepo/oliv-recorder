'use client';

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { useRouter, usePathname } from 'next/navigation';
import { Mic } from 'lucide-react';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useTranscripts } from '@/contexts/TranscriptContext';

// Same key the Home screen uses, so the sensitive choice stays consistent.
const SENSITIVE_KEY = 'oliv_sensitive_meeting';
const COUNTDOWN_SECONDS = 20;

interface Detected {
  app: string;
  bundleId: string;
}

// Listens for `meeting-detected` (emitted by the Rust mic monitor when a
// whitelisted meeting app starts using the mic) and offers to start recording:
// a countdown auto-starts unless cancelled, with the same "Sensitive meeting"
// checkbox as the Home screen.
export default function MeetingDetectedPrompt() {
  const router = useRouter();
  const pathname = usePathname();
  const { isRecording } = useRecordingState();
  const { setMeetingTitle } = useTranscripts();

  const [detected, setDetected] = useState<Detected | null>(null);
  const [seconds, setSeconds] = useState(COUNTDOWN_SECONDS);
  const [sensitive, setSensitive] = useState(false);
  const startedRef = useRef(false);

  // Refs so the (mount-once) event listener reads live values.
  const isRecordingRef = useRef(isRecording);
  const detectedRef = useRef(detected);
  useEffect(() => {
    isRecordingRef.current = isRecording;
  }, [isRecording]);
  useEffect(() => {
    detectedRef.current = detected;
  }, [detected]);

  useEffect(() => {
    const unlisten = listen<Detected>('meeting-detected', (event) => {
      // Ignore if already recording or already prompting.
      if (isRecordingRef.current || detectedRef.current) return;
      let s = false;
      try {
        s = sessionStorage.getItem(SENSITIVE_KEY) === 'true';
      } catch {
        /* sessionStorage unavailable */
      }
      startedRef.current = false;
      setSensitive(s);
      setSeconds(COUNTDOWN_SECONDS);
      setDetected(event.payload);
      invoke('show_main_window').catch(() => {});
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const start = useCallback(
    async (useSensitive: boolean, app: string) => {
      if (startedRef.current) return;
      startedRef.current = true;
      setDetected(null);
      try {
        sessionStorage.setItem(SENSITIVE_KEY, useSensitive ? 'true' : 'false');
      } catch {
        /* sessionStorage unavailable */
      }
      await invoke('oliv_set_sensitive', { sensitive: useSensitive }).catch(() => {});
      await invoke('oliv_set_source_app', { app }).catch(() => {});
      setMeetingTitle(app);
      // Reuse the established start path (same as the tray / sidebar).
      if (pathname === '/') {
        window.dispatchEvent(new CustomEvent('start-recording-from-sidebar'));
      } else {
        try {
          sessionStorage.setItem('autoStartRecording', 'true');
        } catch {
          /* sessionStorage unavailable */
        }
        router.push('/');
      }
    },
    [pathname, router, setMeetingTitle]
  );

  const dismiss = useCallback(() => {
    setDetected(null);
  }, []);

  // Countdown → auto-start on timeout; close if a recording starts meanwhile.
  useEffect(() => {
    if (!detected) return;
    if (isRecording) {
      setDetected(null);
      return;
    }
    if (seconds <= 0) {
      void start(sensitive, detected.app);
      return;
    }
    const t = setTimeout(() => setSeconds((s) => s - 1), 1000);
    return () => clearTimeout(t);
  }, [detected, seconds, isRecording, sensitive, start]);

  if (!detected) return null;

  return (
    <div className="fixed inset-0 z-[9999] flex items-center justify-center bg-black/40">
      <div className="w-[380px] rounded-xl bg-white p-6 shadow-2xl">
        <div className="flex items-center gap-3">
          <span className="flex h-10 w-10 items-center justify-center rounded-full bg-blue-50">
            <Mic className="h-5 w-5 text-blue-600" />
          </span>
          <div>
            <h2 className="text-lg font-semibold text-gray-900">Meeting detected</h2>
            <p className="text-sm text-gray-500">
              <span className="font-medium">{detected.app}</span> is using your microphone.
            </p>
          </div>
        </div>

        <label className="mt-5 flex items-center gap-2 text-sm text-gray-700">
          <input
            type="checkbox"
            checked={sensitive}
            onChange={(e) => setSensitive(e.target.checked)}
            className="h-4 w-4 rounded border-gray-300 text-blue-600 focus:ring-blue-500"
          />
          Sensitive meeting (upload my mic only)
        </label>

        <p className="mt-4 text-sm text-gray-500">
          Recording starts automatically in <span className="font-semibold text-gray-900">{seconds}s</span>…
        </p>

        <div className="mt-5 flex items-center justify-end gap-3">
          <button
            onClick={dismiss}
            className="rounded-lg px-4 py-2 text-sm font-medium text-gray-600 hover:bg-gray-100 transition-colors"
          >
            Dismiss
          </button>
          <button
            onClick={() => start(sensitive, detected.app)}
            className="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 transition-colors"
          >
            Start now
          </button>
        </div>
      </div>
    </div>
  );
}
