import { Channel, invoke } from "@tauri-apps/api/core";
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

export type Settings = {
  excluded_apps: string[];
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
  capture: "off" | "capturing" | "paused";
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
  automatable_hours: number;
  automatable_hours_last_week: number;
  hours_back: number;
};

export type Idea = {
  workflow_id: number;
  workflow: string;
  title: string;
  how: string;
  hours_week: number;
  confidence: number;
};

export type AgentStatus = {
  installed: AgentName[];
  chosen: AgentName | null;
  weekly: boolean;
  running: boolean;
  last_run: number | null;
  last_agent: AgentName | null;
  ideas: Idea[];
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

export type MapRow = {
  id: number;
  name: string;
  steps: string[];
  hours_week: number;
  cases: number;
  action: string;
  rough: boolean;
};

export type Enjoyment = "hate" | "dislike" | "neutral" | "unknown" | "like" | "love";

export type Category =
  | "meeting"
  | "email"
  | "spreadsheet"
  | "crm"
  | "finance"
  | "chat"
  | "planning"
  | "coding"
  | "design"
  | "writing"
  | "slides"
  | "calendar"
  | "social"
  | "media"
  | "web"
  | "other";

export type StepView = {
  key: string;
  name: string;
  gap_s: number | null;
  copies: number;
};

export type WorkflowRow = {
  id: number;
  name: string;
  steps: StepView[];
  hours_week: number;
  cases: number;
  benefit: number;
  procrastination: number;
  action: "automate" | "assist" | "protect" | "ignore" | "watch";
  kind: "friction" | "promise_gap" | "routine" | "long_tail";
  enjoyment: Enjoyment;
  client: boolean;
  rough: boolean;
  handed_off: boolean;
};

export type Overview = {
  hours_back: number;
  total: number;
  ask: WorkflowRow[];
  ranked: WorkflowRow[];
  promises: WorkflowRow[];
  protect: WorkflowRow[];
};

export type Detail = {
  workflow: WorkflowRow;
  reasons: string[];
  copy_points: { from: string; to: string; copies: number }[];
  examples: { entity: string; first_ts: number; last_ts: number; path: string[] }[];
  commitments: { ts: number; source: string; text: string; due_ts: number | null; overdue: boolean }[];
};

export type RuleName =
  | "enjoyment"
  | "client"
  | "not_automatable"
  | "rename"
  | "hide"
  | "merge"
  | "split";
export type Understood = { rule: RuleName; arg: string | null };

export type AutomationPlan = { path: string; prompt: string };

export type ResultRow = {
  handoff_id: number;
  workflow_id: number;
  name: string;
  ts: number;
  status: string;
  note: string;
  days_before: number;
  days_after: number;
  hours_before: number | null;
  hours_after: number | null;
  saved: number | null;
  progress: number;
};

export type Results = { min_days: number; rows: ResultRow[] };

export type SearchPage = {
  sessions: {
    source: "screen" | "mic" | "system";
    app: string;
    title: string;
    url: string;
    start: number;
    end: number;
    lines: { ts: number; text: string }[];
  }[];
};

export type Turn = { id: number; ts: number; question: string; answer: string };

export type Asked = { turn: Turn; page: SearchPage };

/** What `rewind_ask` streams: the sessions once found, then the answer text so far. */
export type AskProgress =
  | { kind: "sessions"; page: SearchPage }
  | { kind: "answer"; text: string };

export type WeeklyReport = {
  from_ts: number;
  to_ts: number;
  hours_back: number;
  hours_saved: number;
  handoffs: number;
  top: { categories: Category[]; hours_week: number; action: string }[];
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
  requestPermission: (name: PromptName) =>
    invoke<boolean>("request_permission", { name }),
  openSettingsPane: (pane: SettingsPane, dragHint?: string) =>
    invoke<void>("open_settings_pane", { pane, dragHint }),
  relaunch: () => invoke<void>("relaunch"),
  pickFolder: (prompt: string) => invoke<string | null>("pick_folder", { prompt }),
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
  workflowOverview: (offset: number, limit: number) =>
    invoke<Overview>("workflow_overview", { offset, limit }),
  answerEnjoyment: (id: number, enjoyment: Enjoyment) =>
    invoke<void>("answer_enjoyment", { id, enjoyment }),
  workflowDetail: (id: number) => invoke<Detail | null>("workflow_detail", { id }),
  correctWorkflow: (id: number, phrase: string, steps: string[]) =>
    invoke<Understood | null>("correct_workflow", { id, phrase, steps }),
  automationPlan: (id: number, folder: string | null) =>
    invoke<AutomationPlan>("automation_plan", { id, folder }),
  writeSkill: (id: number, folder: string | null) =>
    invoke<string>("write_skill", { id, folder }),
  reportHandoff: (id: number) => invoke<void>("report_handoff", { id }),
  results: () => invoke<Results>("results"),
  rewindAsk: (question: string, onProgress: (progress: AskProgress) => void) => {
    const progress = new Channel<AskProgress>();
    progress.onmessage = onProgress;
    return invoke<Asked>("rewind_ask", { question, progress });
  },
  rewindResults: (question: string) =>
    invoke<SearchPage>("rewind_results", { question }),
  rewindHistory: (before: number | null) =>
    invoke<Turn[]>("rewind_history", { before }),
  rewindFit: (height: number) => invoke<void>("rewind_fit", { height }),
  rewindHide: () => invoke<void>("rewind_hide"),
  weeklyReport: () => invoke<WeeklyReport>("weekly_report"),
  saveReportImage: (day: string, png: number[]) =>
    invoke<string>("save_report_image", { day, png }),
  storageUsed: () => invoke<{ bytes: number }>("storage_used"),
  setExcludedApps: (apps: string[]) => invoke<Settings>("set_excluded_apps", { apps }),
  setLive: (live: Live) => invoke<Settings>("set_live", { live }),
  setRetention: (days: number | null) => invoke<Settings>("set_retention", { days }),
  deleteEverything: () => invoke<void>("delete_everything"),
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

/** Calls `handler` each time the shortcut or the menu bar shows the search panel. */
export const onRewindOpen = (handler: () => void): Promise<UnlistenFn> =>
  listen("rewind-open", handler);

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
