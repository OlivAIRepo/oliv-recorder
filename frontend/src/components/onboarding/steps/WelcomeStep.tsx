import React, { useEffect, useState } from 'react';
import { Button } from '@/components/ui/button';
import { OnboardingContainer } from '../OnboardingContainer';
import { useOnboarding } from '@/contexts/OnboardingContext';

export function WelcomeStep() {
  const { goNext, startBackgroundDownloads, completeOnboarding } = useOnboarding();
  const [isMac, setIsMac] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    (async () => {
      try {
        const { platform } = await import('@tauri-apps/plugin-os');
        setIsMac(platform() === 'macos');
      } catch {
        setIsMac(navigator.userAgent.includes('Mac'));
      }
    })();
  }, []);

  const handleStart = async () => {
    if (busy) return;
    setBusy(true);
    // Download the transcription + summarisation engines in the background —
    // never a blocking step. Recording is gated on model-readiness at start time.
    startBackgroundDownloads(true).catch(() => {});
    if (isMac) {
      goNext(); // → Permissions step (mic + system audio)
    } else {
      try {
        await completeOnboarding();
      } finally {
        window.location.reload();
      }
    }
  };

  return (
    <OnboardingContainer
      title="Welcome to Oliv AI"
      description="Transcribe and summarise your meetings without bot"
      step={1}
      hideProgress={true}
    >
      <div className="flex flex-col items-center space-y-10">
        <div className="w-16 h-px bg-gray-300" />
        <div className="w-full max-w-xs">
          <Button
            onClick={handleStart}
            disabled={busy}
            className="w-full h-11 bg-gray-900 hover:bg-gray-800 text-white"
          >
            Get Started
          </Button>
        </div>
      </div>
    </OnboardingContainer>
  );
}
