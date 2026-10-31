import { useCallback, useEffect, useState } from "react";
import {
  api,
  onPopoverOpen,
  type MenuAction,
  type MenuStatus,
  type PermissionName,
} from "./api";
import { duration } from "./format";
import { Pause, Play, Search, AppWindow, Power } from "lucide-react";
import { t, type Key } from "./i18n";
import { Failed } from "./layout";
import Privacy from "./Privacy";
import Today from "./Today";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { cn } from "@/lib/utils";

const POLL_MS = 2000;

const SOURCES: {
  key: keyof MenuStatus["live"];
  label: Key;
  needs?: PermissionName;
}[] = [
  { key: "ocr", label: "sourceScreenText", needs: "screen-recording" },
  { key: "metadata", label: "sourceWindows", needs: "accessibility" },
  { key: "copy", label: "sourceClipboard" },
  { key: "actions", label: "sourceActions", needs: "accessibility" },
  { key: "audio", label: "sourceCalls" },
];

const BLOCKER_LABEL: Partial<Record<PermissionName, Key>> = {
  "screen-recording": "permScreen",
  accessibility: "permAccessibility",
};

type Tone = "on" | "paused" | "blocked" | "off";

function tone(status: MenuStatus): Tone {
  if (status.capture === "capturing")
    return status.blocked_by.length > 0 ? "blocked" : "on";
  return status.capture === "paused" ? "paused" : "off";
}

function statusLine(status: MenuStatus): string {
  switch (tone(status)) {
    case "blocked":
      return t("statusBlocked");
    case "paused":
      return t("statusPaused", {
        time: clock(status.paused_until ?? status.now),
      });
    case "off":
      return t("statusOff");
    case "on": {
      if (status.audio === "transcribing") return t("statusCall");
      if (!status.last_capture || !status.last_app) return t("statusWaiting");
      return t("statusCapturing", {
        app: status.last_app,
        ago: duration(Math.max(0, status.now - status.last_capture)),
      });
    }
  }
}

function clock(ts: number): string {
  return new Date(ts * 1000).toLocaleTimeString(navigator.language, {
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** A dot that breathes while Sextant captures, so you can see at a glance that it runs. */
function Pulse({ tone }: { tone: Tone }) {
  return (
    <span className="relative flex size-2.5 shrink-0" aria-hidden>
      {tone === "on" && (
        <span className="absolute inline-flex size-full animate-ping rounded-full bg-primary opacity-40 [animation-duration:2.4s] motion-reduce:hidden" />
      )}
      <span
        className={cn(
          "relative inline-flex size-2.5 rounded-full",
          tone === "on" && "bg-primary",
          tone === "paused" && "bg-amber-500",
          tone === "blocked" && "bg-destructive",
          tone === "off" && "bg-muted-foreground/50",
        )}
      />
    </span>
  );
}

function Header({
  status,
  onAction,
}: {
  status: MenuStatus;
  onAction: (action: MenuAction) => void;
}) {
  const current = tone(status);
  return (
    <header className="flex flex-col gap-3 px-4 pt-4">
      <div className="flex items-center gap-2.5">
        <Pulse tone={current} />
        <p
          aria-live="polite"
          className="min-w-0 flex-1 truncate text-sm font-medium"
        >
          {statusLine(status)}
        </p>
        {status.capture !== "off" && (
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={status.capture === "paused" ? t("resume") : t("pause")}
            onClick={() => onAction("pause")}
          >
            {status.capture === "paused" ? <Play /> : <Pause />}
          </Button>
        )}
      </div>
      {current === "blocked" && (
        <div className="flex items-center justify-between gap-3 rounded-lg bg-destructive/10 px-3 py-2 text-sm">
          <span>
            {t("blockedBy", {
              names: status.blocked_by
                .map((name) => t(BLOCKER_LABEL[name] ?? "permScreen"))
                .join(", "),
            })}
          </span>
          <Button
            size="xs"
            variant="outline"
            onClick={() =>
              void api.openSettingsPane(
                status.blocked_by[0] === "accessibility"
                  ? "accessibility"
                  : "screen-recording",
                t("dragHint"),
              )
            }
          >
            {t("allow")}
          </Button>
        </div>
      )}
      <ul aria-label={t("capturedNow")} className="flex flex-wrap gap-1.5">
        {SOURCES.map(({ key, label, needs }) => {
          const on =
            status.live[key] &&
            status.capture === "capturing" &&
            !(needs && status.blocked_by.includes(needs));
          return (
            <li
              key={key}
              className={cn(
                "rounded-full px-2 py-0.5 text-xs ring-1",
                on
                  ? "bg-primary/10 text-primary ring-primary/25"
                  : "text-muted-foreground ring-foreground/10 line-through",
              )}
            >
              {t(label)}
            </li>
          );
        })}
      </ul>
    </header>
  );
}

/** The menu bar popover: what Sextant captures now, your day and settings. */
export default function Popover() {
  const [status, setStatus] = useState<MenuStatus | null>(null);
  const [opened, setOpened] = useState(0);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    api
      .menuStatus()
      .then((next) => {
        setStatus(next);
        setError(null);
      })
      .catch((err) => setError(String(err)));
  }, []);

  useEffect(() => {
    refresh();
    const timer = setInterval(() => document.hasFocus() && refresh(), POLL_MS);
    const stop = onPopoverOpen(() => {
      refresh();
      setOpened((n) => n + 1);
    });
    const onKey = (e: KeyboardEvent) =>
      e.key === "Escape" && void api.menuAction("hide");
    window.addEventListener("keydown", onKey);
    return () => {
      clearInterval(timer);
      window.removeEventListener("keydown", onKey);
      void stop.then((unlisten) => unlisten());
    };
  }, [refresh]);

  const act = (action: MenuAction) =>
    api
      .menuAction(action)
      .then(refresh)
      .catch((err) => setError(String(err)));

  return (
    <div className="flex h-full flex-col">
      {status ? (
        <Header status={status} onAction={act} />
      ) : (
        <div className="h-20" />
      )}
      <Tabs defaultValue="today" className="min-h-0 flex-1 gap-0 pt-3">
        <TabsList variant="line" aria-label="Sextant" className="mx-3 shrink-0">
          <TabsTrigger value="today" className="px-2">
            {t("tabToday")}
          </TabsTrigger>
          <TabsTrigger value="settings" className="px-2">
            {t("tabSettings")}
          </TabsTrigger>
        </TabsList>
        <div className="min-h-0 flex-1 overflow-y-auto px-4 py-4">
          <TabsContent value="today" className="flex flex-col gap-5">
            <Today opened={opened} />
          </TabsContent>
          <TabsContent value="settings" className="flex flex-col gap-6">
            <Privacy />
          </TabsContent>
        </div>
      </Tabs>
      {error && (
        <div className="px-4 pb-2">
          <Failed>{error}</Failed>
        </div>
      )}
      <footer className="flex shrink-0 items-center gap-1 border-t border-foreground/10 px-2 py-1.5">
        <Button variant="ghost" size="sm" onClick={() => void act("search")}>
          <Search />
          {t("search")}
          <kbd className="ml-1 text-xs text-muted-foreground">⌘⇧Space</kbd>
        </Button>
        <Button variant="ghost" size="sm" onClick={() => void act("open")}>
          <AppWindow />
          {t("openWindow")}
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          className="ml-auto"
          aria-label={t("quit")}
          onClick={() => void act("quit")}
        >
          <Power />
        </Button>
      </footer>
    </div>
  );
}
