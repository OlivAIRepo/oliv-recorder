'use client';

import { useState, useEffect, useRef } from 'react';
import { motion } from 'framer-motion';
import Image from 'next/image';
import { invoke } from '@tauri-apps/api/core';
import { RecordingControls } from '@/components/RecordingControls';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useRecordingState, RecordingStatus } from '@/contexts/RecordingStateContext';
import { useTranscripts } from '@/contexts/TranscriptContext';
import { useConfig } from '@/contexts/ConfigContext';
import { StatusOverlays } from '@/app/_components/StatusOverlays';
import Analytics from '@/lib/analytics';
import { SettingsModals } from './_components/SettingsModal';
import { useModalState } from '@/hooks/useModalState';
import { useRecordingStateSync } from '@/hooks/useRecordingStateSync';
import { useRecordingStart } from '@/hooks/useRecordingStart';
import { useRecordingStop } from '@/hooks/useRecordingStop';
import { indexedDBService } from '@/services/indexedDBService';
import { DEFAULT_PARAKEET_MODEL } from '@/constants/modelDefaults';
import { toast } from 'sonner';

// Persisted per-session so the audio pipeline (sensitive => upload mic only) can read it.
const SENSITIVE_KEY = 'oliv_sensitive_meeting';

export default function Home() {
  const [isRecording, setIsRecordingState] = useState(false);
  const [barHeights] = useState(['58%', '76%', '58%']);
  const [sensitive, setSensitive] = useState(false);

  const { meetingTitle, setMeetingTitle } = useTranscripts();
  const { transcriptModelConfig, selectedDevices } = useConfig();
  const recordingState = useRecordingState();
  const { status, isProcessing } = recordingState;

  const { setIsMeetingActive, isCollapsed: sidebarCollapsed, refetchMeetings } = useSidebar();
  const { modals, messages, showModal, hideModal } = useModalState(transcriptModelConfig);
  const { isRecordingDisabled, setIsRecordingDisabled } = useRecordingStateSync(
    isRecording, setIsRecordingState, setIsMeetingActive
  );
  const { handleRecordingStart } = useRecordingStart(isRecording, setIsRecordingState, showModal);
  const { handleRecordingStop, setIsStopping } = useRecordingStop(
    setIsRecordingState, setIsRecordingDisabled
  );

  // Resume the transcription-model download if it's missing and not already
  // downloading. The mic/system-audio permission grant forces a macOS
  // quit-and-reopen, which kills an in-progress onboarding download; without
  // this, the app reopens into a limbo state (model incomplete, nothing
  // downloading) where the button looks ready but Start pops the download
  // modal. Re-kicking the download here restores the "Getting you ready…" flow.
  const resumeAttempted = useRef(false);
  useEffect(() => {
    if (resumeAttempted.current) return;
    resumeAttempted.current = true;
    (async () => {
      try {
        await invoke('parakeet_init').catch(() => {});
        const hasModels = await invoke<boolean>('parakeet_has_available_models');
        if (hasModels) return;
        const models = await invoke<{ status: unknown }[]>(
          'parakeet_get_available_models'
        ).catch(() => [] as { status: unknown }[]);
        const downloading = models.some(
          (m) =>
            m.status &&
            typeof m.status === 'object' &&
            'Downloading' in (m.status as Record<string, unknown>)
        );
        if (!downloading) {
          await invoke('parakeet_download_model', {
            modelName: DEFAULT_PARAKEET_MODEL,
          }).catch(() => {});
        }
      } catch {
        /* best-effort; the start flow still gates on model readiness */
      }
    })();
  }, []);

  useEffect(() => {
    Analytics.trackPageView('home');
    let restored = false;
    try {
      restored = sessionStorage.getItem(SENSITIVE_KEY) === 'true';
    } catch { /* sessionStorage unavailable */ }
    setSensitive(restored);
    // Sync the restored value to the backend so the upload decision matches.
    invoke('oliv_set_sensitive', { sensitive: restored }).catch(() => { });
  }, []);

  const toggleSensitive = (val: boolean) => {
    setSensitive(val);
    try {
      sessionStorage.setItem(SENSITIVE_KEY, val ? 'true' : 'false');
    } catch { /* sessionStorage unavailable */ }
    invoke('oliv_set_sensitive', { sensitive: val }).catch(() => { });
  };

  // Startup: prune old local meetings.
  useEffect(() => {
    const performStartupChecks = async () => {
      try {
        if (
          recordingState.isRecording ||
          status === RecordingStatus.STOPPING ||
          status === RecordingStatus.PROCESSING_TRANSCRIPTS ||
          status === RecordingStatus.SAVING
        ) {
          return;
        }
        try { await indexedDBService.deleteOldMeetings(7); } catch (e) { console.warn(e); }
        try { await indexedDBService.deleteSavedMeetings(24); } catch (e) { console.warn(e); }
      } catch (error) {
        console.error('Failed to perform startup checks:', error);
      }
    };
    performStartupChecks();
  }, [recordingState.isRecording, status]);

  // The whole stop → transcribe → save tail: hide the controls cluster and show
  // only the bottom banner; everything returns to normal once it clears.
  const isFinishing =
    status === RecordingStatus.STOPPING ||
    status === RecordingStatus.PROCESSING_TRANSCRIPTS ||
    status === RecordingStatus.SAVING;
  const isProcessingStop = isFinishing || isProcessing;
  const nameValue = meetingTitle === '+ New Call' ? '' : meetingTitle;
  // Always show the start/stop control (except while finishing a recording).
  // Mic permission is handled by the start flow itself — it triggers the OS
  // prompt and surfaces an error if denied — so it must NOT gate the button,
  // otherwise revoking mic access leaves the user with no control at all.
  const controlsVisible = !isFinishing;

  return (
    <motion.div
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.3, ease: 'easeOut' }}
      className="flex flex-col h-screen bg-gray-50"
    >
      <SettingsModals modals={modals} messages={messages} onClose={hideModal} />

      <div className="flex-1 flex flex-col items-center justify-center px-8">
        <div className="w-full max-w-md flex flex-col items-center gap-6">
          <div className="flex flex-col items-center gap-3">
            <Image src="/logo.png" alt="Oliv AI" width={56} height={56} priority />
            <h1 className="text-xl font-semibold text-gray-900">Oliv AI</h1>
          </div>

          <input
            type="text"
            value={nameValue}
            onChange={(e) => setMeetingTitle(e.target.value || '+ New Call')}
            placeholder="Meeting name (optional)"
            disabled={recordingState.isRecording || isProcessingStop}
            className="w-full text-center rounded-lg border border-gray-300 px-4 py-2.5 text-gray-900 placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:bg-gray-100 disabled:text-gray-500"
          />

          <label className="flex items-center gap-2 text-sm text-gray-600 select-none">
            <input
              type="checkbox"
              checked={sensitive}
              onChange={(e) => toggleSensitive(e.target.checked)}
              disabled={recordingState.isRecording || isProcessingStop}
              className="h-4 w-4 rounded border-gray-300"
            />
            Sensitive meeting
            <span className="text-gray-400">— only your voice is transcribed</span>
          </label>

          {controlsVisible && (
            <RecordingControls
              isRecording={recordingState.isRecording}
              onRecordingStop={(callApi = true) => handleRecordingStop(callApi)}
              onRecordingStart={handleRecordingStart}
              onTranscriptReceived={() => { }}
              onStopInitiated={() => setIsStopping(true)}
              barHeights={barHeights}
              onTranscriptionError={(message) => showModal('errorAlert', message)}
              isRecordingDisabled={isRecordingDisabled}
              isParentProcessing={isProcessingStop}
              selectedDevices={selectedDevices}
              meetingName={meetingTitle}
            />
          )}
        </div>
      </div>

      <StatusOverlays
        isProcessing={isFinishing && !recordingState.isRecording}
        isSaving={false}
        sidebarCollapsed={sidebarCollapsed}
      />
    </motion.div>
  );
}
