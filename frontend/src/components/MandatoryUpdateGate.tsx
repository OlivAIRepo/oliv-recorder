'use client';

import React, { useState } from 'react';
import { Download, Loader2, AlertCircle } from 'lucide-react';
import { check } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { UpdateInfo } from '@/services/updateService';

// Full-screen, non-dismissible gate shown when the running version is below the
// published minVersion (a required update). Blocks the app until the user
// installs the update; there is no way to close it.
export function MandatoryUpdateGate({ updateInfo }: { updateInfo: UpdateInfo | null }) {
  const [isUpdating, setIsUpdating] = useState(false);
  const [percentage, setPercentage] = useState(0);
  const [error, setError] = useState<string | null>(null);

  if (!updateInfo?.mandatory) return null;

  const handleUpdate = async () => {
    setIsUpdating(true);
    setError(null);
    setPercentage(0);
    try {
      const update = await check();
      if (!update?.available) {
        setError('Update is no longer available. Please try again.');
        setIsUpdating(false);
        return;
      }
      let downloaded = 0;
      let total = 0;
      await update.downloadAndInstall((event) => {
        if (event.event === 'Started') {
          total = event.data.contentLength || 0;
        } else if (event.event === 'Progress') {
          downloaded += event.data.chunkLength || 0;
          setPercentage(total > 0 ? Math.round((downloaded / total) * 100) : 0);
        } else if (event.event === 'Finished') {
          setPercentage(100);
        }
      });
      await relaunch();
    } catch (err: any) {
      console.error('Mandatory update failed:', err);
      setError(err?.message || 'Update failed. Please try again.');
      setIsUpdating(false);
    }
  };

  return (
    <div className="fixed inset-0 z-[10000] flex items-center justify-center bg-gray-900/80 backdrop-blur-sm">
      <div className="mx-6 w-full max-w-md rounded-2xl bg-white p-7 shadow-2xl text-center">
        <div className="mx-auto mb-4 flex h-12 w-12 items-center justify-center rounded-full bg-blue-50">
          {error ? (
            <AlertCircle className="h-6 w-6 text-red-600" />
          ) : isUpdating ? (
            <Loader2 className="h-6 w-6 animate-spin text-blue-600" />
          ) : (
            <Download className="h-6 w-6 text-blue-600" />
          )}
        </div>

        <h2 className="text-lg font-semibold text-gray-900">Update required</h2>
        <p className="mt-2 text-sm text-gray-600">
          A required update{updateInfo.version ? ` (v${updateInfo.version})` : ''} is available.
          You need to update to keep using Oliv AI.
        </p>

        {isUpdating && (
          <div className="mt-5">
            <div className="h-2.5 w-full rounded-full bg-gray-200">
              <div
                className="h-2.5 rounded-full bg-blue-600 transition-all duration-300"
                style={{ width: `${Math.min(percentage, 100)}%` }}
              />
            </div>
            <p className="mt-2 text-xs text-gray-500">
              {percentage}% — the app will restart automatically when done.
            </p>
          </div>
        )}

        {error && (
          <div className="mt-4 rounded-lg border border-red-200 bg-red-50 p-3">
            <p className="text-sm text-red-800">{error}</p>
          </div>
        )}

        {!isUpdating && (
          <button
            onClick={handleUpdate}
            className="mt-6 inline-flex w-full items-center justify-center gap-2 rounded-lg bg-blue-600 px-4 py-2.5 text-sm font-semibold text-white hover:bg-blue-700 transition-colors"
          >
            <Download className="h-4 w-4" />
            {error ? 'Try again' : 'Update now'}
          </button>
        )}
      </div>
    </div>
  );
}
