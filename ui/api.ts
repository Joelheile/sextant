import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Sources = {
  browsers: boolean;
  safari: boolean;
  calendar: boolean;
  git: boolean;
  files: boolean;
};

export type Live = { metadata: boolean; ocr: boolean; copy: boolean; audio: boolean };

export type Settings = {
  excluded_apps: string[];
  live: Live;
  mcp_rewind: boolean;
  mcp_workflows: boolean;
  onboarded: boolean;
  sources: Sources;
  git_folders: string[];
  call_apps: string[];
};

export type PermissionName =
  | "calendar"
  | "reminders"
  | "safari"
  | "screen-recording"
  | "accessibility"
  | "automation";
export type Permission = {
  name: PermissionName;
  status: "granted" | "undetermined" | "denied";
};

export type SettingsPane =
  | "full-disk-access"
  | "calendars"
  | "reminders"
  | "screen-recording"
  | "accessibility"
  | "automation";

export type MapRow = {
  id: number;
  name: string;
  steps: string[];
  hours_week: number;
  cases: number;
  action: string;
  rough: boolean;
};

export type FirstMap = { hours_back: number; total: number; rows: MapRow[] };

export type ConnectInfo = {
  binary: string;
  claude_code_command: string;
  desktop_config: string;
};

export type Written = { path: string; backup: string | null };

export type Stage =
  | "browsers"
  | "calendar"
  | "git"
  | "files"
  | "saving"
  | "analyzing";

export type BackfillDone = {
  events: number;
  workflows: number;
  hours_back: number;
  sources: { stage: Stage; events: number; errors: string[] }[];
};

export type Models = {
  installed: boolean;
  downloading: boolean;
  audio: boolean;
  bytes: number;
};

export const api = {
  engineStatus: () => invoke<boolean>("engine_status"),
  getState: () => invoke<Settings>("get_state"),
  permissionStatus: () => invoke<Permission[]>("permission_status"),
  requestPermission: (name: PermissionName) =>
    invoke<boolean>("request_permission", { name }),
  openSettingsPane: (pane: SettingsPane) =>
    invoke<void>("open_settings_pane", { pane }),
  pickFolder: () => invoke<string | null>("pick_folder"),
  saveChoices: (sources: Sources, gitFolders: string[]) =>
    invoke<Settings>("save_choices", { sources, gitFolders }),
  startBackfill: () => invoke<void>("start_backfill"),
  firstMap: (offset: number, limit: number) =>
    invoke<FirstMap>("first_map", { offset, limit }),
  connectInfo: () => invoke<ConnectInfo>("connect_info"),
  connectClaudeDesktop: () => invoke<Written>("connect_claude_desktop"),
  setMcpSwitches: (rewind: boolean, workflows: boolean) =>
    invoke<Settings>("set_mcp_switches", { rewind, workflows }),
  modelsStatus: () => invoke<Models>("models_status"),
  downloadModels: () => invoke<void>("download_models"),
  cancelDownload: () => invoke<void>("cancel_download"),
  setAudio: (on: boolean) => invoke<Settings>("set_audio", { on }),
  finishOnboarding: () => invoke<Settings>("finish_onboarding"),
};

/** Calls `handler` when the engine has started or failed to. Resolves once attached. */
export const onReady = (handler: () => void): Promise<UnlistenFn> =>
  listen("ready", handler);

export type BackfillHandlers = {
  progress: (stage: Stage) => void;
  done: (summary: BackfillDone) => void;
  failed: (message: string) => void;
};

/** Listens to the backfill events. Resolves once all three listeners are attached. */
export async function listenBackfill(
  handlers: BackfillHandlers,
): Promise<UnlistenFn> {
  const stops = await Promise.all([
    listen<{ stage: Stage }>("backfill-progress", (e) =>
      handlers.progress(e.payload.stage),
    ),
    listen<BackfillDone>("backfill-done", (e) => handlers.done(e.payload)),
    listen<{ message: string }>("backfill-failed", (e) =>
      handlers.failed(e.payload.message),
    ),
  ]);
  return () => stops.forEach((stop) => stop());
}

export type DownloadHandlers = {
  progress: (done: number, total: number) => void;
  done: () => void;
  failed: (message: string, cancelled: boolean) => void;
};

/** Listens to the model download events. Resolves once all three listeners are attached. */
export async function listenDownload(
  handlers: DownloadHandlers,
): Promise<UnlistenFn> {
  const stops = await Promise.all([
    listen<{ done: number; total: number }>("models-progress", (e) =>
      handlers.progress(e.payload.done, e.payload.total),
    ),
    listen("models-done", handlers.done),
    listen<{ message: string; cancelled: boolean }>("models-failed", (e) =>
      handlers.failed(e.payload.message, e.payload.cancelled),
    ),
  ]);
  return () => stops.forEach((stop) => stop());
}
