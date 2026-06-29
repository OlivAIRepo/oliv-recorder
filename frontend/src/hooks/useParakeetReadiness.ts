'use client';

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useEffect, useRef, useState } from 'react';

export interface ParakeetReadiness {
  /** True once at least one transcription model is available locally. */
  modelReady: boolean;
  /** True while a model is being fetched in the background. */
  isPreparing: boolean;
  /** Download completion 0-100 while preparing, else null. */
  percent: number | null;
}

const POLL_MS = 2000;

/** Pull the download percent out of a ModelStatus, handling both serde shapes:
 *  the struct variant serializes as `{ Downloading: { progress: N } }`, while
 *  the progress event sends a bare number. */
function downloadingPercent(status: unknown): number | null {
  if (!status || typeof status !== 'object') return null;
  if (!('Downloading' in (status as Record<string, unknown>))) return null;
  const d = (status as { Downloading: unknown }).Downloading;
  if (typeof d === 'number') return d;
  if (d && typeof d === 'object' && typeof (d as { progress?: unknown }).progress === 'number') {
    return (d as { progress: number }).progress;
  }
  return 0;
}

/**
 * Tracks whether the local transcription model is ready and, while it's being
 * fetched, how far along the download is. Drives the "Getting you ready…" state
 * on the start button so the user is never shown a blocking setup modal.
 *
 * Polls the same Rust source the tray menu uses (rather than relying on an
 * optimistic default + download events that can be missed by this window), so
 * the home screen and the menubar always agree.
 */
export function useParakeetReadiness(): ParakeetReadiness {
  const [modelReady, setModelReady] = useState(true);
  const [isPreparing, setIsPreparing] = useState(false);
  const [percent, setPercent] = useState<number | null>(null);
  // Latest percent from a live progress event; preferred over the polled value
  // because it updates more smoothly between polls.
  const livePercent = useRef<number | null>(null);

  useEffect(() => {
    let cancelled = false;

    const poll = async () => {
      try {
        await invoke('parakeet_init').catch(() => {});
        const hasModels = await invoke<boolean>('parakeet_has_available_models');
        if (cancelled) return;
        if (hasModels) {
          setModelReady(true);
          setIsPreparing(false);
          setPercent(null);
          livePercent.current = null;
          return;
        }
        setModelReady(false);
        let pct: number | null = null;
        let downloading = false;
        try {
          const models = await invoke<{ status: unknown }[]>(
            'parakeet_get_available_models'
          );
          const dl = models.find((m) => downloadingPercent(m.status) !== null);
          if (dl) {
            downloading = true;
            pct = downloadingPercent(dl.status);
          }
        } catch {
          /* fall back to event-driven preparing state */
        }
        if (cancelled) return;
        setIsPreparing(downloading || livePercent.current !== null);
        setPercent(livePercent.current ?? pct);
      } catch {
        /* keep last state if the engine can't be queried this tick */
      }
    };

    poll();
    const id = setInterval(poll, POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, []);

  useEffect(() => {
    const unlistenProgress = listen<{ progress: number; status?: string }>(
      'parakeet-model-download-progress',
      (event) => {
        const { progress, status } = event.payload;
        if (status === 'completed' || progress >= 100) {
          livePercent.current = null;
          setModelReady(true);
          setIsPreparing(false);
          setPercent(null);
          return;
        }
        if (status === 'cancelled' || status === 'error') {
          livePercent.current = null;
          setIsPreparing(false);
          setPercent(null);
          return;
        }
        livePercent.current = typeof progress === 'number' ? progress : null;
        setModelReady(false);
        setIsPreparing(true);
        if (typeof progress === 'number') setPercent(progress);
      }
    );

    const unlistenComplete = listen('parakeet-model-download-complete', () => {
      livePercent.current = null;
      setModelReady(true);
      setIsPreparing(false);
      setPercent(null);
    });

    return () => {
      unlistenProgress.then((fn) => fn());
      unlistenComplete.then((fn) => fn());
    };
  }, []);

  return { modelReady, isPreparing, percent };
}
