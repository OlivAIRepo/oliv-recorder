import { toast } from 'sonner';

/**
 * Shows a brief "recording started" toast, gated by the user preference.
 * Intentionally minimal — no participant-notification compliance prompt.
 *
 * @returns Promise<void> - Resolves when notification is shown or skipped
 */
export async function showRecordingNotification(): Promise<void> {
  try {
    const { Store } = await import('@tauri-apps/plugin-store');
    const store = await Store.load('preferences.json');
    const showNotification = (await store.get<boolean>('show_recording_notification')) ?? true;

    if (showNotification) {
      toast.info('🔴 Recording started', {
        duration: 4000,
        position: 'bottom-right',
      });
    }
  } catch (notificationError) {
    console.error('Failed to show recording notification:', notificationError);
    // Don't fail the recording if notification fails
  }
}
