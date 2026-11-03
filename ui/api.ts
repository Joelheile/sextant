import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Sources = {
  browsers: boolean;
  safari: boolean;
  calendar: boolean;
  git: boolean;
  files: boolean;
};

export type Live = {
  metadata: boolean;
  ocr: boolean;
  copy: boolean;
  actions: boolean;
  audio: boolean;
  opaque_browsers: boolean;
};

/** One answer an agent received: one MCP tool call, the bridge's included. */
export type Sent = {
  ts: number;
  client: string;
  tool: string;
  bytes: number;
};

export type Settings = {
  excluded_apps: string[];
  excluded_sites: string[];
  storage_cap_mb: number | null;
  live: Live;
  mcp_rewind: boolean;
  mcp_workflows: boolean;
  onboarded: boolean;
  sources: Sources;
  git_folders: string[];
  retention_days: number | null;
  agent: AgentName | null;
  agent_weekly: boolean;
};

export type AgentName = "claude" | "codex";

export type MenuStatus = {
  capture: "off" | "capturing" | "paused" | "stalled";
  paused_until: number | null;
  audio: "off" | "waiting" | "transcribing";
  last_capture: number | null;
  last_app: string | null;
  blocked_by: PermissionName[];
  live: Live;
  now: number;
};

export type Insights = {
  active_minutes: number;
  days: number[];
  apps: { app: string; minutes: number }[];
  switches: number;
  switches_per_hour: number | null;
  longest_focus_minutes: number;
  lines: number;
  call_minutes: number;
};

/** A routine your agent saved, with the hours Sextant measured from what it cites. */
export type Routine = {
  id: number;
  title: string;
  how: string;
  trigger: string;
  approval: string;
  why: string;
  hours_week: number;
  activities: number;
  first: string | null;
  last: string | null;
};

export type AgentStatus = {
  installed: AgentName[];
  chosen: AgentName | null;
  weekly: boolean;
  running: boolean;
  last_run: number | null;
  last_agent: AgentName | null;
  routines: Routine[];
};

export type MenuAction = "pause" | "search" | "open" | "quit" | "hide";

export type PermissionName =
  | "calendar"
  | "reminders"
  | "safari"
  | "screen-recording"
  | "accessibility"
  | "automation";
/** The permissions that have a system prompt. */
export type PromptName = "calendar" | "reminders" | "automation";
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
  tasks: number;
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
  requestPermission: (name: PromptName) =>
    invoke<boolean>("request_permission", { name }),
  openSettingsPane: (pane: SettingsPane, dragHint?: string) =>
    invoke<void>("open_settings_pane", { pane, dragHint }),
  relaunch: () => invoke<void>("relaunch"),
  pickFolder: (prompt: string) =>
    invoke<string | null>("pick_folder", { prompt }),
  saveChoices: (sources: Sources, gitFolders: string[]) =>
    invoke<Settings>("save_choices", { sources, gitFolders }),
  startBackfill: () => invoke<void>("start_backfill"),
  connectInfo: () => invoke<ConnectInfo>("connect_info"),
  connectClaudeDesktop: () => invoke<Written>("connect_claude_desktop"),
  setMcpSwitches: (rewind: boolean, workflows: boolean) =>
    invoke<Settings>("set_mcp_switches", { rewind, workflows }),
  modelsStatus: () => invoke<Models>("models_status"),
  downloadModels: () => invoke<void>("download_models"),
  cancelDownload: () => invoke<void>("cancel_download"),
  setAudio: (on: boolean) => invoke<Settings>("set_audio", { on }),
  finishOnboarding: () => invoke<Settings>("finish_onboarding"),
  storageUsed: () => invoke<{ bytes: number }>("storage_used"),
  debugReport: () => invoke<string>("debug_report"),
  setExcludedApps: (apps: string[]) =>
    invoke<Settings>("set_excluded_apps", { apps }),
  setLive: (live: Live) => invoke<Settings>("set_live", { live }),
  setRetention: (days: number | null) =>
    invoke<Settings>("set_retention", { days }),
  deleteEverything: () => invoke<void>("delete_everything"),
  setExcludedSites: (sites: string[]) =>
    invoke<Settings>("set_excluded_sites", { sites }),
  setStorageCap: (mb: number | null) =>
    invoke<Settings>("set_storage_cap", { mb }),
  forgetRecent: (minutes: number) =>
    invoke<void>("forget_recent", { minutes }),
  sentLog: () => invoke<Sent[]>("sent_log"),
  uninstall: () => invoke<void>("uninstall"),
  menuStatus: () => invoke<MenuStatus>("menu_status"),
  insights: () => invoke<Insights>("insights"),
  menuAction: (action: MenuAction) => invoke<void>("menu_action", { action }),
  agentStatus: () => invoke<AgentStatus>("agent_status"),
  setAgent: (agent: AgentName | null, weekly: boolean) =>
    invoke<Settings>("set_agent", { agent, weekly }),
  runAgent: () => invoke<AgentStatus>("run_agent"),
};

/** Calls `handler` when the engine has started or failed to. Resolves once attached. */
export const onReady = (handler: () => void): Promise<UnlistenFn> =>
  listen("ready", handler);

/** Calls `handler` each time the menu bar popover opens. */
export const onPopoverOpen = (handler: () => void): Promise<UnlistenFn> =>
  listen("popover-open", handler);

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
