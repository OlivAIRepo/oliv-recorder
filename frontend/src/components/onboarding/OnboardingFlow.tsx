import React, { useEffect } from 'react';
import { useOnboarding } from '@/contexts/OnboardingContext';
import { WelcomeStep, PermissionsStep } from './steps';

interface OnboardingFlowProps {
  onComplete: () => void;
}

export function OnboardingFlow(_props: OnboardingFlowProps) {
  const { currentStep } = useOnboarding();
  const [isMac, setIsMac] = React.useState(false);

  useEffect(() => {
    const checkPlatform = async () => {
      try {
        const { platform } = await import('@tauri-apps/plugin-os');
        setIsMac(platform() === 'macos');
      } catch (e) {
        console.error('Failed to detect platform:', e);
        setIsMac(navigator.userAgent.includes('Mac'));
      }
    };
    checkPlatform();
  }, []);

  // Simplified onboarding:
  //   Step 1: Welcome — kicks off engine downloads in the BACKGROUND.
  //   Step 2: Permissions (macOS only) — mic + system audio.
  // The old "setup overview" and blocking "download progress" steps are gone.
  // Non-macOS finishes at Welcome (no permission step needed).
  return (
    <div className="onboarding-flow">
      {(currentStep === 1 || !isMac) && <WelcomeStep />}
      {currentStep >= 2 && isMac && <PermissionsStep />}
    </div>
  );
}
