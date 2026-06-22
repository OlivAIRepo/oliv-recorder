'use client'

import React, { createContext, useContext, useState, useCallback, useEffect, useRef } from 'react';
import { useUpdateCheck } from '@/hooks/useUpdateCheck';
import { UpdateInfo } from '@/services/updateService';
import { UpdateDialog } from './UpdateDialog';
import { MandatoryUpdateGate } from './MandatoryUpdateGate';
import { setUpdateDialogCallback, showUpdateNotification } from './UpdateNotification';

interface UpdateCheckContextType {
  updateInfo: UpdateInfo | null;
  isChecking: boolean;
  checkForUpdates: (force?: boolean) => Promise<UpdateInfo | null>;
  showUpdateDialog: () => void;
  /** User-initiated check (e.g. Settings button): opens the dialog directly on
   *  an available update (no toast) and returns the result for inline feedback. */
  checkNow: () => Promise<UpdateInfo | null>;
}

const UpdateCheckContext = createContext<UpdateCheckContextType | undefined>(undefined);

export function UpdateCheckProvider({ children }: { children: React.ReactNode }) {
  const [showDialog, setShowDialog] = useState(false);
  // True while a user-initiated check is running, so we open the dialog directly
  // instead of showing the passive toast.
  const interactiveRef = useRef(false);

  const handleShowDialog = useCallback(() => {
    setShowDialog(true);
  }, []);

  const { updateInfo, isChecking, checkForUpdates } = useUpdateCheck({
    checkOnMount: true,
    showNotification: true,
    onUpdateAvailable: (info) => {
      // Mandatory updates are handled by the blocking gate — no dismissible toast.
      if (info.mandatory) return;
      // User-initiated check → open the dialog directly (no redundant toast).
      if (interactiveRef.current) {
        setShowDialog(true);
        return;
      }
      // Passive (on-launch) check → non-intrusive toast.
      showUpdateNotification(info, handleShowDialog);
    },
  });

  const checkNow = useCallback(async () => {
    interactiveRef.current = true;
    try {
      return await checkForUpdates(true);
    } finally {
      interactiveRef.current = false;
    }
  }, [checkForUpdates]);

  useEffect(() => {
    // Register the callback so UpdateNotification can trigger the dialog
    setUpdateDialogCallback(handleShowDialog);
    return () => {
      setUpdateDialogCallback(() => {});
    };
  }, [handleShowDialog]);

  // Listen for tray menu events
  useEffect(() => {
    const handleTrayCheck = () => {
      checkForUpdates(true); // Force check from tray
      setShowDialog(true);
    };

    window.addEventListener('check-updates-from-tray', handleTrayCheck);
    return () => window.removeEventListener('check-updates-from-tray', handleTrayCheck);
  }, [checkForUpdates]);

  return (
    <UpdateCheckContext.Provider
      value={{
        updateInfo,
        isChecking,
        checkForUpdates,
        showUpdateDialog: handleShowDialog,
        checkNow,
      }}
    >
      {children}
      <UpdateDialog
        open={showDialog}
        onOpenChange={setShowDialog}
        updateInfo={updateInfo}
      />
      {/* Blocking gate for required (below-minVersion) updates. */}
      <MandatoryUpdateGate updateInfo={updateInfo} />
    </UpdateCheckContext.Provider>
  );
}

export function useUpdateCheckContext() {
  const context = useContext(UpdateCheckContext);
  if (context === undefined) {
    throw new Error('useUpdateCheckContext must be used within UpdateCheckProvider');
  }
  return context;
}
