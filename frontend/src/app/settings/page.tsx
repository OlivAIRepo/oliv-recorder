'use client';

import React, { useCallback, useEffect, useState } from 'react';
import { ArrowLeft, LogIn, CheckCircle2, AlertTriangle, Loader2, RefreshCw } from 'lucide-react';
import { useRouter } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getVersion } from '@tauri-apps/api/app';
import { OLIV_LOGIN_URL } from '@/lib/olivAuth';
import { useConfig } from '@/contexts/ConfigContext';
import { usePermissionCheck } from '@/hooks/usePermissionCheck';
import { useUpdateCheckContext } from '@/components/UpdateCheckProvider';
import { DeviceSelection } from '@/components/DeviceSelection';

// One permission's real state + a Grant button when it's missing.
function PermissionRow({
  label,
  granted,
  onGrant,
}: {
  label: string;
  granted: boolean;
  onGrant: () => void;
}) {
  return (
    <div className="flex items-center justify-between gap-3">
      <div className="flex items-center gap-2 text-sm">
        {granted ? (
          <CheckCircle2 className="w-5 h-5 text-green-600" />
        ) : (
          <AlertTriangle className="w-5 h-5 text-amber-500" />
        )}
        <span className="text-gray-700">{label}</span>
        <span className={granted ? 'text-green-600' : 'text-amber-600'}>
          {granted ? 'Granted' : 'Not granted'}
        </span>
      </div>
      {!granted && (
        <button
          onClick={onGrant}
          className="rounded-lg bg-blue-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-blue-700 transition-colors"
        >
          Grant
        </button>
      )}
    </div>
  );
}

export default function SettingsPage() {
  const router = useRouter();
  const [account, setAccount] = useState<{ email: string } | null>(null);
  const { selectedDevices, setSelectedDevices } = useConfig();
  const { hasMicrophone, isChecking, checkPermissions } = usePermissionCheck();
  const { checkNow, isChecking: isCheckingUpdate } = useUpdateCheckContext();
  const [appVersion, setAppVersion] = useState<string>('');
  const [upToDate, setUpToDate] = useState(false);

  useEffect(() => {
    getVersion().then(setAppVersion).catch(() => {});
  }, []);

  const handleCheckUpdates = useCallback(async () => {
    setUpToDate(false);
    const info = await checkNow();
    // If an update is available the provider opens the dialog; otherwise confirm.
    if (info && !info.available) setUpToDate(true);
  }, [checkNow]);
  // System-audio recording permission. Prefer the real OS state (non-prompting
  // screen/system-audio-recording preflight); fall back to the persisted flag.
  // Using the real check fixes the case where it's granted at the OS level but a
  // stale flag showed "Not granted".
  const [audioGranted, setAudioGranted] = useState(false);

  const refreshAudioGranted = useCallback(async () => {
    // Trust ONLY the real OS check. A persisted "granted" flag used to be OR'd
    // in here, but it was sticky-true across reinstalls and revokes — so it
    // falsely reported "Granted" while capture silently produced no audio.
    let osGranted = false;
    try {
      osGranted = await invoke<boolean>('check_screen_recording_permission_command');
    } catch {
      /* command unavailable */
    }
    setAudioGranted(osGranted);
  }, []);

  useEffect(() => {
    refreshAudioGranted();
  }, [refreshAudioGranted]);

  const grantSystemAudio = async () => {
    // Starts the same Core Audio tap recording uses → triggers the Audio Capture
    // prompt when the permission is still undetermined.
    await invoke<boolean>('trigger_system_audio_permission_command').catch(() => false);
    await refreshAudioGranted();
    // If it's still not granted (e.g. previously denied — macOS won't re-prompt),
    // open System Settings so the user can enable it manually.
    const granted = await invoke<boolean>('check_screen_recording_permission_command').catch(
      () => false
    );
    if (!granted) {
      await invoke('open_screen_recording_settings_command').catch(() => {});
    }
    // Starting the Core Audio / ScreenCaptureKit tap briefly steals foreground;
    // pull our window back to front so the user lands back in Settings.
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      await getCurrentWindow().setFocus();
    } catch {
      /* not in a Tauri window */
    }
  };

  const refreshAccount = useCallback(() => {
    invoke<{ email: string } | null>('get_oliv_account')
      .then((acct) => setAccount(acct ?? null))
      .catch(() => setAccount(null));
  }, []);

  useEffect(() => {
    refreshAccount();
    const unlistenPromise = listen('oliv-auth-changed', () => refreshAccount());
    return () => {
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, [refreshAccount]);

  const handleLogin = async () => {
    try {
      await invoke('open_external_url', { url: OLIV_LOGIN_URL });
    } catch (error) {
      console.error('Failed to open Oliv login:', error);
    }
  };

  const handleLogout = async () => {
    try {
      await invoke('oliv_logout');
    } catch (error) {
      console.error('Failed to log out:', error);
    } finally {
      refreshAccount();
    }
  };

  return (
    <div className="h-screen bg-gray-50 flex flex-col">
      <div className="sticky top-0 z-10 bg-gray-50 border-b border-gray-200">
        <div className="max-w-2xl mx-auto px-8 py-6">
          <div className="flex items-center gap-4">
            <button
              onClick={() => router.back()}
              className="flex items-center gap-2 text-gray-600 hover:text-gray-900 transition-colors"
            >
              <ArrowLeft className="w-5 h-5" />
              <span>Back</span>
            </button>
            <h1 className="text-3xl font-bold">Settings</h1>
          </div>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        <div className="max-w-2xl mx-auto p-8 pt-6">
          <div className="bg-white rounded-xl border border-gray-200 p-6">
            <h2 className="text-lg font-semibold text-gray-900">Account</h2>
            {account ? (
              <div className="mt-4 flex items-center justify-between gap-3">
                <div className="flex items-center gap-3 text-gray-700">
                  <CheckCircle2 className="w-5 h-5 text-green-600" />
                  <span className="font-medium">Logged in</span>
                </div>
                <button
                  onClick={handleLogout}
                  className="text-sm text-gray-500 hover:text-gray-800 transition-colors"
                >
                  Log out
                </button>
              </div>
            ) : (
              <>
                <p className="mt-1 text-sm text-gray-500">
                  Sign in with your Oliv account to sync recordings.
                </p>
                <button
                  onClick={handleLogin}
                  className="mt-4 inline-flex items-center gap-2 rounded-lg bg-blue-600 px-4 py-2.5 text-white font-medium hover:bg-blue-700 transition-colors"
                >
                  <LogIn className="w-4 h-4" />
                  Login with Oliv
                </button>
              </>
            )}
          </div>

          {/* Permissions */}
          <div className="mt-6 bg-white rounded-xl border border-gray-200 p-6">
            <h2 className="text-lg font-semibold text-gray-900">Permissions</h2>
            <p className="mt-1 text-sm text-gray-500">
              Oliv needs microphone and system-audio (screen recording) access to record meetings.
            </p>
            <div className="mt-4 space-y-3">
              {isChecking ? (
                <p className="text-sm text-gray-500">Checking permissions…</p>
              ) : (
                <>
                  <PermissionRow
                    label="Microphone"
                    granted={hasMicrophone}
                    onGrant={async () => {
                      await invoke('trigger_microphone_permission').catch(() => {});
                      setTimeout(checkPermissions, 1000);
                    }}
                  />
                  <PermissionRow
                    label="System audio recording"
                    granted={audioGranted}
                    onGrant={grantSystemAudio}
                  />
                </>
              )}
            </div>
          </div>

          {/* Audio devices */}
          <div className="mt-6 bg-white rounded-xl border border-gray-200 p-6">
            <h2 className="text-lg font-semibold text-gray-900">Audio devices</h2>
            <p className="mt-1 text-sm text-gray-500">
              Choose which microphone and speaker (system audio) to record.
            </p>
            <div className="mt-4">
              <DeviceSelection
                selectedDevices={selectedDevices}
                onDeviceChange={setSelectedDevices}
              />
            </div>
          </div>

          {/* Updates */}
          <div className="mt-6 bg-white rounded-xl border border-gray-200 p-6">
            <h2 className="text-lg font-semibold text-gray-900">Updates</h2>
            <p className="mt-1 text-sm text-gray-500">
              Current version{appVersion ? `: ${appVersion}` : ''}
            </p>
            <div className="mt-4">
              {upToDate && !isCheckingUpdate && (
                <p className="mb-2 inline-flex items-center gap-1.5 text-sm text-green-600">
                  <CheckCircle2 className="w-4 h-4" />
                  You're on the latest version
                </p>
              )}
              <button
                onClick={handleCheckUpdates}
                disabled={isCheckingUpdate}
                className="flex w-full items-center justify-center gap-2 rounded-lg border border-gray-200 px-4 py-2.5 text-sm font-medium text-gray-700 hover:bg-gray-50 transition-colors disabled:opacity-60 disabled:cursor-not-allowed"
              >
                {isCheckingUpdate ? (
                  <>
                    <Loader2 className="w-4 h-4 animate-spin" />
                    Checking…
                  </>
                ) : (
                  <>
                    <RefreshCw className="w-4 h-4" />
                    Check for updates
                  </>
                )}
              </button>
            </div>
          </div>

          {/* Reset */}
          <div className="mt-6 bg-white rounded-xl border border-gray-200 p-6">
            <h2 className="text-lg font-semibold text-gray-900">Reset</h2>
            <p className="mt-1 text-sm text-gray-500">
              Log out and clear local app data (onboarding + downloaded models). The app
              restarts and re-downloads the models. Your recordings are kept.
            </p>
            <button
              onClick={() => window.dispatchEvent(new CustomEvent('oliv-open-reset'))}
              className="mt-4 inline-flex items-center gap-2 rounded-lg border border-red-200 bg-red-50 px-4 py-2.5 text-sm font-medium text-red-700 hover:bg-red-100 transition-colors"
            >
              Reset app data &amp; log out
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
