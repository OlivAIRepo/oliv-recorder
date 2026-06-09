'use client';

import React, { useCallback, useEffect, useState } from 'react';
import { ArrowLeft, LogIn, CheckCircle2 } from 'lucide-react';
import { useRouter } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

// my.oliv.ai login. We point it at the same-origin my.oliv.ai/recorder-auth
// bridge page (autosched-mirror) which, after login, redirects to
// olivrecorder://auth-callback?ic_token=... The Rust deep-link handler (auth.rs)
// then persists the token to the OS keychain and emits `oliv-auth-changed`,
// which we listen for below.
// Use `redirect=` (NOT `final-page=`): Login.tsx drives the post-login client
// redirect off the `redirect` query param; `final-page` is ignored on that path.
const OLIV_LOGIN_URL =
  'https://my.oliv.ai/login?redirect=' +
  encodeURIComponent('https://my.oliv.ai/recorder-auth');

export default function SettingsPage() {
  const router = useRouter();
  const [account, setAccount] = useState<{ email: string } | null>(null);

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
        </div>
      </div>
    </div>
  );
}
