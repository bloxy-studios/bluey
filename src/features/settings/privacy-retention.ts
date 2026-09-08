/** Raw-audio retention window (Privacy → Raw audio → Custom window). */

export const RAW_AUDIO_RETENTION_MIN_MINUTES = 1;
export const RAW_AUDIO_RETENTION_MAX_MINUTES = 240;
export const RAW_AUDIO_RETENTION_DEFAULT_MINUTES = 30;

export function clampRetentionMinutes(value: number): number {
  if (!Number.isFinite(value)) return RAW_AUDIO_RETENTION_DEFAULT_MINUTES;
  return Math.min(
    RAW_AUDIO_RETENTION_MAX_MINUTES,
    Math.max(RAW_AUDIO_RETENTION_MIN_MINUTES, Math.round(value)),
  );
}
