import { invoke } from '@tauri-apps/api/core';

/**
 * Typed wrappers over Tauri commands. Components never call `invoke` directly; they call store
 * actions, which call these. Types come from `./generated/` (written by `cargo test -p sc-core`).
 */

/** The application version as reported by the Rust side. */
export function appVersion(): Promise<string> {
  return invoke<string>('app_version');
}
