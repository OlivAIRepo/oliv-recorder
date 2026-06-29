'use client';

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useCallback, useEffect, useState } from 'react';

export interface ParakeetReadiness {
  /** True once at least one transcription model is available locally. */
  modelReady: boolean;
  /** True while a model is being fetched in the background. */
  isPreparing: boolean;
  /** Download completion 0-100 while preparing, else null. */
  percent: number | null;
  /** Estimated seconds remaining while preparing, else null. */
  etaSeconds: number | null;
}

/**
 * Tracks whether the local transcription model is ready and, while it is being
 * fetched, how far along the download is. Drives the "Getting you ready…" state
 * on the start button so the user is never shown a blocking setup modal.
 *
 * Starts optimistic (modelReady=true) so the button isn't disabled before the
 * first async check resolves; the on-mount refresh corrects it immediately.
 */
export function useParakeetReadiness(): ParakeetReadiness {
  const [modelReady, setModelReady] = useState(true);
  const [isPreparing, setIsPreparing] = useState(false);
  const [percent, setPercent] = useState<number | null>(null);
  const [etaSeconds, setEtaSeconds] = useState<number | null>(null);

  const markReady = useCallback(() => {
    setModelReady(true);
    setIsPreparing(false);
    setPercent(null);
    setEtaSeconds(null);
  }, []);

  const refresh = useCallback(async () => {
    try {
      await invoke('parakeet_init').catch(() => {});
      const hasModels = await invoke<boolean>('parakeet_has_available_models');
      if (hasModels) {
        markReady();
        return;
      }
      setModelReady(false);
      // If a download is already underway (started before mount), reflect it.
      try {
        const models = await invoke<{ status: unknown }[]>(
          'parakeet_get_available_models'
        );
        const downloading = models.find(
          (m) =>
            m.status &&
            typeof m.status === 'object' &&
            'Downloading' in (m.status as Record<string, unknown>)
        );
        if (downloading) {
          const p = (downloading.status as { Downloading: number }).Downloading;
          setIsPreparing(true);
          setPercent(typeof p === 'number' ? p : null);
        }
      } catch {
        /* leave preparing state to the progress listener */
      }
    } catch {
      /* keep optimistic default if the engine can't be queried */
    }
  }, [markReady]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  useEffect(() => {
    const unlistenProgress = listen<{
      progress: number;
      downloaded_mb?: number;
      total_mb?: number;
      speed_mbps?: number;
      status?: string;
    }>('parakeet-model-download-progress', (event) => {
      const { progress, downloaded_mb, total_mb, speed_mbps, status } =
        event.payload;

      if (status === 'completed' || progress >= 100) {
        markReady();
        return;
      }
      if (status === 'cancelled' || status === 'error') {
        setIsPreparing(false);
        setPercent(null);
        setEtaSeconds(null);
        refresh();
        return;
      }

      setModelReady(false);
      setIsPreparing(true);
      setPercent(typeof progress === 'number' ? progress : null);

      const remainingMb = (total_mb ?? 0) - (downloaded_mb ?? 0);
      setEtaSeconds(
        speed_mbps && speed_mbps > 0 && remainingMb > 0
          ? remainingMb / speed_mbps
          : null
      );
    });

    const unlistenComplete = listen('parakeet-model-download-complete', () => {
      markReady();
    });

    const unlistenError = listen('parakeet-model-download-error', () => {
      setIsPreparing(false);
      setPercent(null);
      setEtaSeconds(null);
      refresh();
    });

    return () => {
      unlistenProgress.then((fn) => fn());
      unlistenComplete.then((fn) => fn());
      unlistenError.then((fn) => fn());
    };
  }, [markReady, refresh]);

  return { modelReady, isPreparing, percent, etaSeconds };
}
