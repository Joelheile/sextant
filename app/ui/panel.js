// The menu bar panel. Every value comes from the app through the `ask` command and goes into the
// page with textContent, so captured text never becomes markup.
//
// The week reads left to right as a funnel: what Sextant captured, the tasks it found in it, the
// work that repeats and the routines your agent saved. Picking a stage shows what is in it.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const REFRESH_MS = 3000;
const ENTRANCE_MS = 500;
const HOUR_S = 3600;
const WEEK_DAYS = 7;
const MAX_ROWS = 5;
const SVG = "http://www.w3.org/2000/svg";

// The funnel narrows by a fixed shape: its stages count different things, so they share no scale.
const FUNNEL = { width: 360, height: 124, bar: 14, heights: [116, 84, 56, 34] };
const STAGES = [
  { key: "captured", label: "Captured" },
  { key: "tasks", label: "Tasks" },
  { key: "repeats", label: "Repeats" },
  { key: "routines", label: "Routines" },
];

const PERMISSIONS = {
  "screen-recording": ["Screen Recording", "Read the text on screen."],
  accessibility: [
    "Accessibility",
    "Read the focused app, window and controls.",
  ],
  automation: ["Automation", "Read the address of the browser tab."],
};

// Screen Recording applies only after Sextant opens again.
let needsRelaunch = false;
let isSetUp = false;
let hasSeenIntro = false;
let chosen = null;
let week = null;
let agent = null;

const $ = (id) => document.getElementById(id);

function ask(cmd, args = {}) {
  return invoke("ask", { ask: { cmd, ...args } });
}

function nowSeconds() {
  return Math.floor(Date.now() / 1000);
}

function duration(minutes) {
  const whole = Math.round(minutes);
  if (whole < 1) return "under 1 min";
  if (whole < 60) return `${whole} min`;
  const rest = whole % 60;
  return rest === 0
    ? `${whole / 60} h`
    : `${Math.floor(whole / 60)} h ${rest} min`;
}

function hours(minutes) {
  return minutes < 60
    ? `${Math.round(minutes)} min`
    : `${Math.round(minutes / 60)} h`;
}

function ago(ts) {
  const seconds = Math.max(nowSeconds() - ts, 0);
  if (seconds < 10) return "just now";
  if (seconds < 60) return `${seconds} s ago`;
  if (seconds < HOUR_S) return `${Math.floor(seconds / 60)} min ago`;
  return `${Math.floor(seconds / HOUR_S)} h ago`;
}

function bytes(count) {
  const mb = count / 1e6;
  return mb < 1000
    ? `${Math.round(mb)} MB on this Mac`
    : `${(mb / 1000).toFixed(1)} GB on this Mac`;
}

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function svg(tag, attributes) {
  const node = document.createElementNS(SVG, tag);
  for (const [name, value] of Object.entries(attributes)) {
    node.setAttribute(name, value);
  }
  return node;
}

function replay(node, className) {
  node.classList.remove(className);
  void node.offsetWidth;
  node.classList.add(className);
}

function enter() {
  document
    .querySelectorAll(".reveal")
    .forEach((node, index) => node.style.setProperty("--i", index));
  replay(document.body, "entering");
  setTimeout(() => document.body.classList.remove("entering"), ENTRANCE_MS);
}

// The window is as tall as the page, so the glass ends where the content does.
let fitted = 0;
function fit() {
  const height = Math.ceil(document.body.getBoundingClientRect().height);
  if (height === fitted) return;
  fitted = height;
  ask("fit", { height }).catch(showError);
}

// Header.

function showStatus(status) {
  const isPaused = status.paused_until !== null;
  const isCapturing = status.line.startsWith("Capturing");
  const isOff = status.line === "Capture is off";
  const isBlocked =
    status.problem !== null || !(isPaused || isCapturing || isOff);
  $("line").textContent = status.line;
  $("pulse").className =
    `pulse ${isBlocked ? "alert" : isCapturing ? "on" : ""}`;
  $("now").textContent = status.last_capture
    ? `${status.last_capture[1]}, ${ago(status.last_capture[0])}`
    : "";
  $("pause").textContent = isPaused ? "Resume" : "Pause 1 h";
  $("pause").disabled = status.problem !== null || isOff;
  $("stop-audio").hidden = status.audio === "off";
  $("problem").hidden = status.problem === null;
  $("problem").textContent = status.problem ?? "";
  $("storage").textContent = bytes(status.storage_bytes);
  // After setup, a permission turned off in System Settings blocks capture.
  isSetUp = status.set_up;
  const needsSetup = !isSetUp || status.line.startsWith("Capture is blocked");
  $("intro").hidden = !needsSetup || isSetUp || hasSeenIntro;
  $("setup").hidden = !needsSetup || !$("intro").hidden;
  $("week").hidden = needsSetup;
  $("finish-setup").textContent = isSetUp ? "Open again" : "Start capture";
}

// Funnel.

function stageValues() {
  const repeats = week.repeated_tasks.length + week.repeated_activities.length;
  return {
    captured: hours(week.active_minutes),
    tasks: week.task_count.toLocaleString(),
    repeats: repeats.toLocaleString(),
    routines: agent.routines.length.toLocaleString(),
  };
}

function stageCounts() {
  return {
    captured: week.active_minutes,
    tasks: week.task_count,
    repeats: week.repeated_tasks.length + week.repeated_activities.length,
    routines: agent.routines.length,
  };
}

function bandPath(x1, h1, x2, h2) {
  const cy = FUNNEL.height / 2;
  const mid = (x1 + x2) / 2;
  const [top1, top2, bottom1, bottom2] = [
    cy - h1 / 2,
    cy - h2 / 2,
    cy + h1 / 2,
    cy + h2 / 2,
  ];
  return `M${x1} ${top1} C${mid} ${top1} ${mid} ${top2} ${x2} ${top2} L${x2} ${bottom2} C${mid} ${bottom2} ${mid} ${bottom1} ${x1} ${bottom1} Z`;
}

// Draws the shape; `counts` marks empty stages, `null` draws the empty funnel of the intro.
function funnelShape(counts) {
  const { width, height, bar, heights } = FUNNEL;
  const column = width / STAGES.length;
  const center = (index) => column * (index + 0.5);
  const shape = svg("svg", {
    viewBox: `0 0 ${width} ${height}`,
    class: "shape",
    "aria-hidden": "true",
  });
  for (let index = 1; index < STAGES.length; index += 1) {
    const path = bandPath(
      center(index - 1) + bar / 2,
      heights[index - 1],
      center(index) - bar / 2,
      heights[index],
    );
    shape.append(svg("path", { d: path, class: "band" }));
  }
  STAGES.forEach((stage, index) => {
    const isEmpty = counts === null || counts[stage.key] === 0;
    shape.append(
      svg("rect", {
        x: center(index) - bar / 2,
        y: (height - heights[index]) / 2,
        width: bar,
        height: heights[index],
        rx: 5,
        class: `bar s${index} ${isEmpty ? "none" : ""} ${stage.key === chosen ? "chosen" : ""}`,
      }),
    );
  });
  return shape;
}

function stageButton(stage, value) {
  const button = element("button", "stage-tab");
  button.setAttribute("role", "tab");
  button.setAttribute("aria-selected", String(stage.key === chosen));
  button.dataset.stage = stage.key;
  button.append(
    element("b", "value", value),
    element("span", "label", stage.label),
  );
  return button;
}

function showFunnel() {
  const values = stageValues();
  const counts = stageCounts();
  if (chosen === null) {
    chosen =
      ["routines", "repeats", "tasks"].find((key) => counts[key] > 0) ??
      "captured";
  }
  const columns = element("div", "columns");
  columns.append(
    ...STAGES.map((stage) => stageButton(stage, values[stage.key])),
  );
  $("funnel").replaceChildren(funnelShape(counts), columns);
  showStage();
}

// Stage detail.

function listRow(name, aside) {
  const row = element("li", "item");
  row.append(element("span", "name", name), aside);
  return row;
}

function dayDots(days) {
  const dots = element("span", "dots");
  for (let index = 0; index < WEEK_DAYS; index += 1) {
    dots.append(element("i", index < days ? "hit" : ""));
  }
  return dots;
}

function capturedBody() {
  const most = Math.max(...week.days.map((day) => day.active_minutes), 1);
  const chart = element("ol", "days");
  for (const day of week.days) {
    const column = element("li");
    const fill = element("span", "fill");
    fill.style.height = `${Math.max((day.active_minutes / most) * 100, 2)}%`;
    column.title = `${day.weekday}: ${duration(day.active_minutes)}`;
    column.append(fill, element("span", "caption", day.weekday.slice(0, 2)));
    chart.append(column);
  }
  return [chart];
}

function tasksBody() {
  const list = element("ol", "list");
  list.append(
    ...week.top_tasks.map((task) =>
      listRow(task.label, element("span", "caption", duration(task.minutes))),
    ),
  );
  const more = week.task_count - week.top_tasks.length;
  return more > 0
    ? [list, element("p", "caption", `and ${more} more`)]
    : [list];
}

function repeatsBody() {
  const repeats = [
    ...week.repeated_tasks.map((repeat) => ({ name: repeat.label, ...repeat })),
    ...week.repeated_activities.map((repeat) => ({
      name: repeat.label,
      ...repeat,
    })),
  ]
    .sort((a, b) => b.minutes - a.minutes)
    .slice(0, MAX_ROWS);
  const list = element("ol", "list");
  list.append(
    ...repeats.map((repeat) => listRow(repeat.name, dayDots(repeat.days))),
  );
  return [list];
}

function routinesBody() {
  const list = element("ol", "list");
  list.append(
    ...agent.routines.map((routine) => {
      const row = listRow(
        routine.title,
        element("span", "caption", `${routine.hours_week.toFixed(1)} h a week`),
      );
      row.title = routine.why;
      return row;
    }),
  );
  return [list];
}

const STAGE_NOTES = {
  captured: () => `Active ${duration(week.active_minutes)}, by day`,
  tasks: () => "Work grouped by what it was about, longest first",
  repeats: () => "Done on 2 or more days, with the days it happened",
  routines: () =>
    agent.running
      ? "Your agent is looking for routines"
      : "Saved by your agent",
};

const EMPTY = {
  captured: "Nothing captured this week yet.",
  tasks: "Tasks show up once Sextant sees related work.",
  repeats: "Repeats show up once work happens on two days.",
  routines: "Your agent saves routines here from what repeats.",
};

const BODIES = {
  captured: capturedBody,
  tasks: tasksBody,
  repeats: repeatsBody,
  routines: routinesBody,
};

function showStage() {
  const isEmpty = stageCounts()[chosen] === 0;
  $("stage-note").textContent = STAGE_NOTES[chosen]();
  $("stage-body").replaceChildren(
    ...(isEmpty ? [element("p", "caption", EMPTY[chosen])] : BODIES[chosen]()),
  );
  const hasRepeats = stageCounts().repeats > 0;
  $("find").hidden =
    chosen !== "routines" || agent.chosen === null || !hasRepeats;
  $("find").disabled = agent.running;
  $("find").textContent = agent.running ? "Finding routines" : "Find routines";
}

function choose(key) {
  if (key === chosen) return;
  chosen = key;
  showFunnel();
  replay($("stage"), "arrive");
  fit();
}

// Setup.

function permissionRow(permission) {
  const [label, why] = PERMISSIONS[permission.name];
  const row = element("li", "item");
  const text = element("span", "name");
  text.append(element("b", "", label), element("span", "caption", why));
  row.append(text);
  if (permission.status === "granted") {
    row.append(element("span", "allowed", "Allowed"));
    return row;
  }
  const button = element("button", "pill", "Allow");
  button.addEventListener("click", () => allow(permission.name));
  row.append(button);
  return row;
}

function allow(name) {
  if (name === "automation") {
    act("automation");
    return;
  }
  if (name === "screen-recording") needsRelaunch = true;
  ask("allow", { pane: name }).catch(showError);
}

async function showSetup() {
  const permissions = await ask("permissions");
  $("permissions").replaceChildren(...permissions.map(permissionRow));
}

function beginSetup() {
  hasSeenIntro = true;
  refresh();
}

function finishSetup() {
  if (isSetUp) {
    ask("relaunch").catch(showError);
    return;
  }
  ask("finish-setup").then(
    () => (needsRelaunch ? ask("relaunch") : refresh()),
    showError,
  );
}

// Loading.

function showError(error) {
  $("problem").hidden = false;
  $("problem").textContent = String(error);
}

async function refresh() {
  try {
    const status = await ask("status");
    showStatus(status);
    if (!$("setup").hidden) await showSetup();
    // Until the engine has started there is nothing to read.
    if (status.problem === null && !$("week").hidden) {
      [week, agent] = await Promise.all([
        ask("week", { at: nowSeconds() }),
        ask("routines"),
      ]);
      showFunnel();
    }
  } catch (error) {
    showError(error);
  }
  fit();
}

function act(cmd) {
  ask(cmd).then(refresh, showError);
}

function showIntroFunnel() {
  const columns = element("div", "columns");
  columns.append(
    ...STAGES.map((stage) => {
      const column = element("span", "stage-tab");
      column.append(element("span", "label", stage.label));
      return column;
    }),
  );
  $("intro").querySelector(".funnel").replaceChildren(funnelShape(null), columns);
}

showIntroFunnel();
$("funnel").addEventListener("click", (event) => {
  const button = event.target.closest("[data-stage]");
  if (button) choose(button.dataset.stage);
});
$("pause").addEventListener("click", () =>
  act($("pause").textContent === "Resume" ? "resume" : "pause"),
);
$("stop-audio").addEventListener("click", () => act("stop-audio"));
$("find").addEventListener("click", () => act("find-routines"));
$("begin-setup").addEventListener("click", beginSetup);
$("finish-setup").addEventListener("click", finishSetup);
$("quit").addEventListener("click", () => act("quit"));

listen("panel-open", () => {
  enter();
  refresh();
});
setInterval(() => document.hasFocus() && refresh(), REFRESH_MS);
enter();
refresh();
