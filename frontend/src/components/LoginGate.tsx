'use client';

import React, { useCallback, useEffect, useState } from 'react';
import Image from 'next/image';
import { LogIn } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { OLIV_LOGIN_URL } from '@/lib/olivAuth';

type AuthState = 'loading' | 'authed' | 'unauthed';

// Gates the main app behind Oliv login. Rendered AFTER onboarding, so the
// first-run model setup is never blocked. When not logged in, only the login
// screen is shown — no other page is reachable.
export default function LoginGate({ children }: { children: React.ReactNode }) {
  const [state, setState] = useState<AuthState>('loading');

  const refresh = useCallback(() => {
    invoke<{ email: string } | null>('get_oliv_account')
      .then((acct) => setState(acct ? 'authed' : 'unauthed'))
      .catch(() => setState('unauthed'));
  }, []);

  useEffect(() => {
    refresh();
    const unlisten = listen('oliv-auth-changed', () => refresh());
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [refresh]);

  const handleLogin = useCallback(async () => {
    try {
      await invoke('open_external_url', { url: OLIV_LOGIN_URL });
    } catch (error) {
      console.error('Failed to open Oliv login:', error);
    }
  }, []);

  if (state === 'authed') {
    return <>{children}</>;
  }

  if (state === 'loading') {
    return <div className="h-screen bg-gray-50" />;
  }

  return (
    <div className="h-screen bg-gray-50 flex flex-col items-center justify-center gap-6 px-8">
      <Image src="/logo-collapsed.png" alt="Oliv" width={56} height={45} priority />
      <div className="text-center">
        <h1 className="text-2xl font-bold text-gray-900">Welcome to Oliv AI</h1>
        <p className="mt-2 text-sm text-gray-500">
          Sign in with your Oliv account to continue.
        </p>
      </div>
      <button
        onClick={handleLogin}
        className="inline-flex items-center gap-2 rounded-lg bg-blue-600 px-5 py-3 text-white font-medium hover:bg-blue-700 transition-colors"
      >
        <LogIn className="w-4 h-4" />
        Login with Oliv
      </button>
    </div>
  );
}
