/**
 * Update Service
 *
 * Handles automatic software updates using Tauri updater plugin.
 * Provides update checking, downloading, and installation functionality.
 */

import { check, Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { getVersion } from '@tauri-apps/api/app';
import { invoke } from '@tauri-apps/api/core';

export interface UpdateInfo {
  available: boolean;
  currentVersion: string;
  version?: string;
  date?: string;
  body?: string;
  downloadUrl?: string;
  /** True when the current version is below the published minVersion → the
   *  update is required and the app should block until it's installed. */
  mandatory?: boolean;
}

/** Compare dotted numeric versions ("0.3.0", "1.2.0.4"). -1 if a<b, 0, 1 if a>b. */
export function compareVersions(a: string, b: string): number {
  const pa = a.split('.').map((n) => parseInt(n, 10) || 0);
  const pb = b.split('.').map((n) => parseInt(n, 10) || 0);
  const len = Math.max(pa.length, pb.length);
  for (let i = 0; i < len; i++) {
    const x = pa[i] ?? 0;
    const y = pb[i] ?? 0;
    if (x !== y) return x < y ? -1 : 1;
  }
  return 0;
}

export interface UpdateProgress {
  downloaded: number;
  total: number;
  percentage: number;
}

/**
 * Update Service
 * Singleton service for managing app updates
 */
export class UpdateService {
  private updateCheckInProgress = false;
  private lastCheckTime: number | null = null;
  private readonly CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000; // 24 hours

  /**
   * Check for available updates
   * @param force Force check even if recently checked
   * @returns Promise with update information
   */
  async checkForUpdates(force = false): Promise<UpdateInfo> {
    // Prevent concurrent update checks
    if (this.updateCheckInProgress) {
      throw new Error('Update check already in progress');
    }

    // Skip if checked recently (unless forced)
    if (!force && this.lastCheckTime) {
      const timeSinceLastCheck = Date.now() - this.lastCheckTime;
      if (timeSinceLastCheck < this.CHECK_INTERVAL_MS) {
        console.log('Skipping update check - checked recently');
        return {
          available: false,
          currentVersion: await getVersion(),
        };
      }
    }

    this.updateCheckInProgress = true;
    this.lastCheckTime = Date.now();

    try {
      const currentVersion = await getVersion();
      const update = await check();

      if (update?.available) {
        // Forced update (Option B): current version below the published
        // minVersion. Fetched in Rust; null/failure → not mandatory.
        let mandatory = false;
        try {
          const minVersion = await invoke<string | null>('oliv_min_version');
          if (minVersion) {
            mandatory = compareVersions(currentVersion, minVersion) < 0;
          }
        } catch {
          /* no manifest / offline → optional */
        }
        return {
          available: true,
          currentVersion,
          version: update.version,
          date: update.date,
          body: update.body,
          mandatory,
        };
      }

      return {
        available: false,
        currentVersion,
        mandatory: false,
      };
    } catch (error) {
      console.error('Failed to check for updates:', error);
      throw error;
    } finally {
      this.updateCheckInProgress = false;
    }
  }

  /**
   * Download and install the available update
   * @param update The update object from checkForUpdates
   * @param onProgress Optional progress callback
   * @returns Promise that resolves when download completes
   */
  async downloadAndInstall(
    update: Update,
    onProgress?: (progress: UpdateProgress) => void
  ): Promise<void> {
    try {
      // Download the update
      await update.download();

      // Notify progress if callback provided
      if (onProgress) {
        onProgress({ downloaded: 100, total: 100, percentage: 100 });
      }

      // Install and relaunch
      await update.install();
      await relaunch();
    } catch (error) {
      console.error('Failed to download/install update:', error);
      throw error;
    }
  }

  /**
   * Get the current app version
   * @returns Promise with version string
   */
  async getCurrentVersion(): Promise<string> {
    return getVersion();
  }

  /**
   * Check if an update check was performed recently
   * @returns true if checked within the interval
   */
  wasCheckedRecently(): boolean {
    if (!this.lastCheckTime) return false;
    const timeSinceLastCheck = Date.now() - this.lastCheckTime;
    return timeSinceLastCheck < this.CHECK_INTERVAL_MS;
  }
}

// Export singleton instance
export const updateService = new UpdateService();
