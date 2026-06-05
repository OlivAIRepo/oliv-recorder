'use client';

import { useState, useEffect } from 'react';
import { motion } from 'framer-motion';
import Image from 'next/image';
import { RecordingControls } from '@/components/RecordingControls';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { usePermissionCheck } from '@/hooks/usePermissionCheck';
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
import { useTranscriptRecovery } from '@/hooks/useTranscriptRecovery';
import { TranscriptRecovery } from '@/components/TranscriptRecovery';
import { indexedDBService } from '@/services/indexedDBService';
import { toast } from 'sonner';

// Persisted per-session so the audio pipeline (sensitive => upload mic only) can read it.
const SENSITIVE_KEY = 'oliv_sensitive_meeting';

export default function Home() {
  const [isRecording, setIsRecordingState] = useState(false);
  const [barHeights] = useState(['58%', '76%', '58%']);
  const [showRecoveryDialog, setShowRecoveryDialog] = useState(false);
  const [sensitive, setSensitive] = useState(false);

  const { meetingTitle, setMeetingTitle } = useTranscripts();
  const { transcriptModelConfig, selectedDevices } = useConfig();
  const recordingState = useRecordingState();
  const { status, isProcessing } = recordingState;

  const { hasMicrophone } = usePermissionCheck();
  const { setIsMeetingActive, isCollapsed: sidebarCollapsed, refetchMeetings } = useSidebar();
  const { modals, messages, showModal, hideModal } = useModalState(transcriptModelConfig);
  const { isRecordingDisabled, setIsRecordingDisabled } = useRecordingStateSync(
    isRecording, setIsRecordingState, setIsMeetingActive
  );
  const { handleRecordingStart } = useRecordingStart(isRecording, setIsRecordingState, showModal);
  const { handleRecordingStop, setIsStopping } = useRecordingStop(
    setIsRecordingState, setIsRecordingDisabled
  );

  const {
    recoverableMeetings,
    checkForRecoverableTranscripts,
    recoverMeeting,
    loadMeetingTranscripts,
    deleteRecoverableMeeting,
  } = useTranscriptRecovery();

  useEffect(() => {
    Analytics.trackPageView('home');
    try {
      setSensitive(sessionStorage.getItem(SENSITIVE_KEY) === 'true');
    } catch { /* sessionStorage unavailable */ }
  }, []);

  const toggleSensitive = (val: boolean) => {
    setSensitive(val);
    try {
      sessionStorage.setItem(SENSITIVE_KEY, val ? 'true' : 'false');
    } catch { /* sessionStorage unavailable */ }
  };

  // Startup: prune old local meetings + offer crash recovery.
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
        await checkForRecoverableTranscripts();
      } catch (error) {
        console.error('Failed to perform startup checks:', error);
      }
    };
    performStartupChecks();
  }, [checkForRecoverableTranscripts, recordingState.isRecording, status]);

  useEffect(() => {
    if (recoverableMeetings.length > 0) {
      const shownThisSession = sessionStorage.getItem('recovery_dialog_shown');
      if (!shownThisSession) {
        setShowRecoveryDialog(true);
        sessionStorage.setItem('recovery_dialog_shown', 'true');
      }
    }
  }, [recoverableMeetings]);

  const handleRecovery = async (meetingId: string) => {
    try {
      const result = await recoverMeeting(meetingId);
      if (result.success) {
        toast.success('Meeting recovered.', { duration: 6000 });
        await refetchMeetings();
        if (recoverableMeetings.length === 0) {
          sessionStorage.removeItem('recovery_dialog_shown');
        }
      }
    } catch (error) {
      toast.error('Failed to recover meeting', {
        description: error instanceof Error ? error.message : 'Unknown error occurred',
      });
      throw error;
    }
  };

  const handleDialogClose = () => {
    setShowRecoveryDialog(false);
    if (recoverableMeetings.length === 0) {
      sessionStorage.removeItem('recovery_dialog_shown');
    }
  };

  const isProcessingStop = status === RecordingStatus.PROCESSING_TRANSCRIPTS || isProcessing;
  const nameValue = meetingTitle === '+ New Call' ? '' : meetingTitle;
  const controlsVisible =
    (hasMicrophone || isRecording) &&
    status !== RecordingStatus.PROCESSING_TRANSCRIPTS &&
    status !== RecordingStatus.SAVING;

  return (
    <motion.div
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.3, ease: 'easeOut' }}
      className="flex flex-col h-screen bg-gray-50"
    >
      <SettingsModals modals={modals} messages={messages} onClose={hideModal} />

      <TranscriptRecovery
        isOpen={showRecoveryDialog}
        onClose={handleDialogClose}
        recoverableMeetings={recoverableMeetings}
        onRecover={handleRecovery}
        onDelete={deleteRecoverableMeeting}
        onLoadPreview={loadMeetingTranscripts}
      />

      <div className="flex-1 flex flex-col items-center justify-center px-8">
        <div className="w-full max-w-md flex flex-col items-center gap-6">
          <div className="flex flex-col items-center gap-3">
            <Image src="/logo.png" alt="Oliv Recorder" width={56} height={56} priority />
            <h1 className="text-xl font-semibold text-gray-900">Oliv Recorder</h1>
          </div>

          <input
            type="text"
            value={nameValue}
            onChange={(e) => setMeetingTitle(e.target.value || '+ New Call')}
            placeholder="Meeting name (optional)"
            disabled={recordingState.isRecording || isProcessingStop}
            className="w-full text-center rounded-lg border border-gray-300 px-4 py-2.5 text-gray-900 placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:bg-gray-100 disabled:text-gray-500"
          />

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

          <label className="flex items-center gap-2 text-sm text-gray-600 select-none">
            <input
              type="checkbox"
              checked={sensitive}
              onChange={(e) => toggleSensitive(e.target.checked)}
              disabled={recordingState.isRecording || isProcessingStop}
              className="h-4 w-4 rounded border-gray-300"
            />
            Sensitive meeting
            <span className="text-gray-400">— only your mic is uploaded</span>
          </label>
        </div>
      </div>

      <StatusOverlays
        isProcessing={status === RecordingStatus.PROCESSING_TRANSCRIPTS && !recordingState.isRecording}
        isSaving={status === RecordingStatus.SAVING}
        sidebarCollapsed={sidebarCollapsed}
      />
    </motion.div>
  );
}
