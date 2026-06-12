/**
 * Intentionally a no-op: we do not surface any "recording started" toast.
 * Kept as a stable entry point so callers (useRecordingStart) don't change.
 */
export async function showRecordingNotification(): Promise<void> {
  // No notification by design.
}
