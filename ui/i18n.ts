const en = {
  next: "Next",
  finish: "Finish",
  copy: "Copy",
  copied: "Copied",

  removeFolder: "Remove",

  accessNone: "The sources you picked need no permission.",
  permScreen: "Screen Recording (text in the window you use)",
  permAccessibility: "Accessibility (window titles)",
  permAutomation: "Automation (browser address and private windows)",
  granted: "Allowed",
  undetermined: "Not asked yet",
  denied: "Not allowed",
  allow: "Allow",
  openSettings: "Open System Settings",
  allowAll: "Allow all",
  dragHint: "Drag Sextant into the list, then switch it on.",
  reopenHint:
    "Screen Recording starts working after Sextant reopens.",
  reopen: "Reopen Sextant",
  automationHint:
    "Open your browser first, then press Allow. Without this, Sextant skips browser windows.",
  hoursWeek: "{hours} h a week",

  desktopAdd: "Add to Claude Desktop",
  desktopDone:
    "Added. Your old config is saved as {backup}. Restart Claude Desktop.",
  desktopDoneNoBackup: "Added to {path}. Restart Claude Desktop.",
  cloudNote:
    "Whatever the agent reads runs where its model runs. With a cloud model, that can be the cloud.",
  switchRewind: "Agents can search what I saw and said",
  switchWorkflows: "Agents can read my activities, tasks and routines",

  audioTitle: "Transcribe calls",
  audioBody:
    "When a call app such as Zoom, Teams or Meet uses your microphone, Sextant can write down what you and the other side say. The sound is never saved: only the text stays, on this Mac.",
  audioDownload:
    "This needs a one-time download of about {size} MB from github.com and huggingface.co. Nothing downloads until you press the button. Sextant makes no other network connection.",
  audioCredit:
    "Speech recognition: Parakeet TDT 0.6B v3 by NVIDIA (CC BY 4.0, ONNX version by istupakov). Voice detection: Silero VAD (MIT). Runtime: ONNX Runtime (MIT).",
  audioLaw:
    "In Germany, recording a call without the other side knowing can be a crime (section 201 StGB). Tell everyone on a call that you transcribe it.",
  audioDownloadButton: "Download and turn on",
  audioSwitch: "Transcribe calls while a call app uses the microphone",
  downloading: "{done} of {total} MB",
  downloadFailed: "The download failed: {message}",
  cancel: "Cancel",
  aboutTitle: "About and licenses",
  aboutBody:
    "Speech recognition uses Parakeet TDT 0.6B v3 by NVIDIA, licensed CC BY 4.0, in an ONNX version by istupakov. Voice detection uses Silero VAD and speech runs on ONNX Runtime, both MIT licensed.",

  loading: "Loading",
  errorTitle: "Something went wrong",

  privacySources: "Sources and permissions",
  liveMetadata: "Window titles and app names",
  liveOcr: "Text on screen",
  liveCopy: "Which app copied (never what)",
  liveActions:
    "Buttons you click and what you leave in text fields (never keystrokes or passwords)",
  liveOpaque: "Browsers that cannot report private windows",
  excludedTitle: "Excluded apps",
  excludedBody: "Text from these apps is never shown to Claude or in search.",
  excludedAdd: "Add",
  excludedPlaceholder: "App name",
  sitesTitle: "Excluded sites",
  sitesBody:
    "Pages on these sites and their subdomains are never read, live or from browser history. Banks and payment sites are on the list from the start.",
  sitesPlaceholder: "example.com",
  capTitle: "Storage limit",
  cap_1000: "1 GB",
  cap_2000: "2 GB",
  cap_5000: "5 GB",
  cap_10000: "10 GB",
  cap_none: "No limit",
  forgetTitle: "Delete recent data",
  forgetBody:
    "Deletes everything captured in that time: screen text, actions, transcripts and searches.",
  forget_15: "Last 15 minutes",
  forget_60: "Last hour",
  forget_1440: "Last day",
  forgotten: "Deleted",
  sentTitle: "Sent to your agent",
  sentBody:
    "Each answer an agent got from Sextant, with its size. The content itself is not kept here.",
  sentEmpty: "Nothing sent yet.",
  sentRow: "{time} · {client} · {tool} · {size} KB",
  uninstallTitle: "Uninstall Sextant",
  uninstallBody:
    "Deletes everything, removes Sextant from Claude Desktop, moves the app to the Trash and quits. If you added Sextant to Claude Code, also run: claude mcp remove sextant",
  uninstallButton: "Uninstall",
  uninstallConfirm: "This deletes all your data and the app. Uninstall?",
  uninstallYes: "Yes, uninstall",
  uninstallWorking: "Uninstalling",
  retentionTitle: "Keep data for",
  retention_30: "30 days",
  retention_90: "90 days",
  retention_365: "1 year",
  retention_forever: "Forever",
  storageTitle: "Storage used",
  storageValue: "{size} MB on this Mac",
  debugTitle: "Debug report",
  debugBody:
    "Versions, switches, counts and error kinds. It holds nothing you saw, typed or said. Paste it into a GitHub issue.",
  debugCopy: "Copy debug report",
  mcpTitle: "What agents can read",
  deleteTitle: "Delete everything",
  deleteBody:
    "Stops capture, deletes all data and downloaded models, removes the key from the Keychain and starts Sextant fresh.",
  deleteButton: "Delete everything",
  deleteConfirm: "This cannot be undone. Delete everything?",
  deleteYes: "Yes, delete everything",
  deleteWorking: "Deleting",
  routinesTitle: "Routines your agent found",
  routinesTotal: "a week in your saved routines",
  routinesFrom: "From {agent}, {time}",
  routinesNone:
    "Your agent reads your weeks in Sextant and saves the work it could take over. Sextant measures the hours from what it cites.",
  routineStop: "Asks you before: {action}",
  routineSeen: "{count} activities, {first} to {last}",
  routineSkill: "To turn it into a skill, paste this into {agent}:",
  analyzeWith: "Find routines with {agent}",
  analyzing: "Your agent is reading your weeks, this takes a few minutes",
  pickAgent: "Pick Claude Code or Codex in Settings to find routines.",
  bridgeTitle: "Agent",
  bridgeBody:
    "Sextant runs your own Claude Code or Codex, signed in with your own plan. Sextant opens no connection itself: the agent does, only when it runs.",
  bridgeAgent: "Agent",
  bridgeOff: "Off",
  notInstalled: "{agent} (not installed)",
  bridgeWeekly: "Look for routines once a week",
  bridgeSent:
    "It reads your weeks through Sextant's tools, as allowed under \"What agents can read\", and \"Sent to your agent\" lists every answer it gets. Claude gets no tools but Sextant's. Codex has no such mode, so it runs read-only and offline in an empty folder, without your config, plugins or web search.",
} as const;

export type Key = keyof typeof en;

const de: Record<Key, string> = {
  next: "Weiter",
  finish: "Fertig",
  copy: "Kopieren",
  copied: "Kopiert",

  removeFolder: "Entfernen",

  accessNone: "Die gewählten Quellen brauchen keine Erlaubnis.",
  permScreen: "Bildschirmaufnahme (Text im Fenster, das du nutzt)",
  permAccessibility: "Bedienungshilfen (Fenstertitel)",
  permAutomation: "Automation (Browser-Adresse und private Fenster)",
  granted: "Erlaubt",
  undetermined: "Noch nicht gefragt",
  denied: "Nicht erlaubt",
  allow: "Erlauben",
  openSettings: "Systemeinstellungen öffnen",
  allowAll: "Alle erlauben",
  dragHint: "Zieh Sextant in die Liste und schalte es dann ein.",
  reopenHint:
    "Bildschirmaufnahme wirkt erst, wenn Sextant neu startet.",
  reopen: "Sextant neu öffnen",
  automationHint:
    "Öffne zuerst deinen Browser und tippe dann auf Erlauben. Ohne diese Erlaubnis überspringt Sextant Browserfenster.",
  hoursWeek: "{hours} Std. pro Woche",

  desktopAdd: "Zu Claude Desktop hinzufügen",
  desktopDone:
    "Hinzugefügt. Deine alte Konfiguration liegt als {backup}. Starte Claude Desktop neu.",
  desktopDoneNoBackup: "Hinzugefügt in {path}. Starte Claude Desktop neu.",
  cloudNote:
    "Was der Agent liest, läuft dort, wo sein Modell läuft. Bei einem Cloud-Modell kann das die Cloud sein.",
  switchRewind: "Agenten dürfen durchsuchen, was ich gesehen und gesagt habe",
  switchWorkflows:
    "Agenten dürfen meine Aktivitäten, Aufgaben und Routinen lesen",

  audioTitle: "Gespräche mitschreiben",
  audioBody:
    "Wenn eine Anruf-App wie Zoom, Teams oder Meet dein Mikrofon nutzt, kann Sextant mitschreiben, was du und die Gegenseite sagen. Der Ton wird nie gespeichert: Nur der Text bleibt, auf diesem Mac.",
  audioDownload:
    "Dafür ist einmalig ein Download von etwa {size} MB von github.com und huggingface.co nötig. Es lädt nichts, bevor du die Taste drückst. Sextant baut keine andere Netzwerkverbindung auf.",
  audioCredit:
    "Spracherkennung: Parakeet TDT 0.6B v3 von NVIDIA (CC BY 4.0, ONNX-Version von istupakov). Erkennung von Sprechpausen: Silero VAD (MIT). Laufzeit: ONNX Runtime (MIT).",
  audioLaw:
    "In Deutschland kann es strafbar sein, ein Gespräch ohne Wissen der Gegenseite aufzuzeichnen (§ 201 StGB). Sag allen im Gespräch, dass du es mitschreibst.",
  audioDownloadButton: "Herunterladen und einschalten",
  audioSwitch:
    "Gespräche mitschreiben, solange eine Anruf-App das Mikrofon nutzt",
  downloading: "{done} von {total} MB",
  downloadFailed: "Der Download ist fehlgeschlagen: {message}",
  cancel: "Abbrechen",
  aboutTitle: "Über Sextant und Lizenzen",
  aboutBody:
    "Die Spracherkennung nutzt Parakeet TDT 0.6B v3 von NVIDIA unter CC BY 4.0, in einer ONNX-Version von istupakov. Die Sprachpausen erkennt Silero VAD, die Modelle laufen auf ONNX Runtime, beide unter MIT-Lizenz.",

  loading: "Lädt",
  errorTitle: "Etwas ist schiefgelaufen",

  privacySources: "Quellen und Berechtigungen",
  liveMetadata: "Fenstertitel und App-Namen",
  liveOcr: "Text auf dem Bildschirm",
  liveCopy: "Welche App kopiert hat (nie was)",
  liveActions:
    "Angeklickte Schaltflächen und was in Textfeldern steht (nie Tastenanschläge oder Passwörter)",
  liveOpaque: "Browser, die private Fenster nicht melden",
  excludedTitle: "Ausgeschlossene Apps",
  excludedBody:
    "Text aus diesen Apps wird nie an Claude oder in der Suche gezeigt.",
  excludedAdd: "Hinzufügen",
  excludedPlaceholder: "App-Name",
  sitesTitle: "Ausgeschlossene Websites",
  sitesBody:
    "Seiten dieser Websites und ihrer Subdomains werden nie gelesen, weder live noch aus dem Browserverlauf. Banken und Bezahldienste stehen von Anfang an auf der Liste.",
  sitesPlaceholder: "beispiel.de",
  capTitle: "Speichergrenze",
  cap_1000: "1 GB",
  cap_2000: "2 GB",
  cap_5000: "5 GB",
  cap_10000: "10 GB",
  cap_none: "Keine Grenze",
  forgetTitle: "Letzte Daten löschen",
  forgetBody:
    "Löscht alles, was in dieser Zeit erfasst wurde: Bildschirmtext, Aktionen, Transkripte und Suchen.",
  forget_15: "Letzte 15 Minuten",
  forget_60: "Letzte Stunde",
  forget_1440: "Letzter Tag",
  forgotten: "Gelöscht",
  sentTitle: "An deinen Agenten gesendet",
  sentBody:
    "Jede Antwort, die ein Agent von Sextant bekam, mit ihrer Größe. Der Inhalt selbst wird hier nicht gespeichert.",
  sentEmpty: "Noch nichts gesendet.",
  sentRow: "{time} · {client} · {tool} · {size} KB",
  uninstallTitle: "Sextant deinstallieren",
  uninstallBody:
    "Löscht alles, entfernt Sextant aus Claude Desktop, legt die App in den Papierkorb und beendet sie. Wenn du Sextant zu Claude Code hinzugefügt hast, führe auch aus: claude mcp remove sextant",
  uninstallButton: "Deinstallieren",
  uninstallConfirm: "Das löscht alle deine Daten und die App. Deinstallieren?",
  uninstallYes: "Ja, deinstallieren",
  uninstallWorking: "Wird deinstalliert",
  retentionTitle: "Daten aufbewahren für",
  retention_30: "30 Tage",
  retention_90: "90 Tage",
  retention_365: "1 Jahr",
  retention_forever: "Für immer",
  storageTitle: "Speicherplatz",
  storageValue: "{size} MB auf diesem Mac",
  debugTitle: "Fehlerbericht",
  debugBody:
    "Versionen, Schalter, Zahlen und Fehlerarten. Er enthält nichts, was du gesehen, getippt oder gesagt hast. Füge ihn in ein GitHub-Issue ein.",
  debugCopy: "Fehlerbericht kopieren",
  mcpTitle: "Was Agenten lesen dürfen",
  deleteTitle: "Alles löschen",
  deleteBody:
    "Stoppt die Aufnahme, löscht alle Daten und geladenen Modelle, entfernt den Schlüssel aus dem Schlüsselbund und startet Sextant neu.",
  deleteButton: "Alles löschen",
  deleteConfirm: "Das lässt sich nicht rückgängig machen. Alles löschen?",
  deleteYes: "Ja, alles löschen",
  deleteWorking: "Wird gelöscht",
  routinesTitle: "Routinen, die dein Agent gefunden hat",
  routinesTotal: "pro Woche in deinen gespeicherten Routinen",
  routinesFrom: "Von {agent}, {time}",
  routinesNone:
    "Dein Agent liest deine Wochen in Sextant und speichert die Arbeit, die er übernehmen könnte. Sextant misst die Stunden an dem, was er belegt.",
  routineStop: "Fragt dich vor: {action}",
  routineSeen: "{count} Aktivitäten, {first} bis {last}",
  routineSkill: "Für einen Skill füge das in {agent} ein:",
  analyzeWith: "Routinen mit {agent} finden",
  analyzing: "Dein Agent liest deine Wochen, das dauert ein paar Minuten",
  pickAgent:
    "Wähle in den Einstellungen Claude Code oder Codex, um Routinen zu finden.",
  bridgeTitle: "Agent",
  bridgeBody:
    "Sextant startet dein eigenes Claude Code oder Codex, angemeldet mit deinem eigenen Abo. Sextant selbst baut keine Verbindung auf: Das tut der Agent, nur während er läuft.",
  bridgeAgent: "Agent",
  bridgeOff: "Aus",
  notInstalled: "{agent} (nicht installiert)",
  bridgeWeekly: "Einmal pro Woche nach Routinen suchen",
  bridgeSent:
    "Er liest deine Wochen über die Werkzeuge von Sextant, so weit du es unter \"Was Agenten lesen dürfen\" erlaubst, und \"An deinen Agenten gesendet\" listet jede Antwort, die er bekommt. Claude bekommt nur die Werkzeuge von Sextant. Codex kennt diesen Modus nicht und läuft deshalb nur lesend und offline in einem leeren Ordner, ohne deine Konfiguration, Plugins oder Websuche.",
};

const strings: Record<Key, string> = navigator.language
  .toLowerCase()
  .startsWith("de")
  ? de
  : en;

/** `t("hoursWeek", { hours: 3.5 })` fills {hours} in the string for the system language. */
export function t(
  key: Key,
  values: Record<string, string | number> = {},
): string {
  return strings[key].replace(/\{(\w+)\}/g, (_, name: string) =>
    String(values[name] ?? ""),
  );
}
