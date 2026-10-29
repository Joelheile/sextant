import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { api, onRewindOpen, type SearchPage, type Turn } from "./api";
import { dateTime } from "./format";
import { Search } from "lucide-react";
import { t } from "./i18n";
import { Failed, Muted } from "./layout";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

/** Matches `REWIND_PAGE` in `app/src/screens.rs`. */
const HISTORY_PAGE = 20;

type Entry = { turn: Turn; page: SearchPage | null };

/** The search panel: past questions scroll up like a chat, the box sits at the bottom. */
export default function Rewind() {
  const [entries, setEntries] = useState<Entry[] | null>(null);
  const [older, setOlder] = useState(false);
  const [query, setQuery] = useState("");
  /** How far back Up has gone through past questions, or null while typing. */
  const [recall, setRecall] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [asking, setAsking] = useState(false);
  const panel = useRef<HTMLDivElement>(null);
  const log = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  /** Distance from the bottom of the log to keep when older turns are added on top. */
  const keepBottom = useRef<number | null>(0);

  const loadHistory = useCallback(async () => {
    try {
      const turns = await api.rewindHistory(null);
      keepBottom.current = 0;
      setEntries(turns.reverse().map((turn) => ({ turn, page: null })));
      setOlder(turns.length === HISTORY_PAGE);
      setError(null);
    } catch (err) {
      setError(String(err));
    }
  }, []);

  useEffect(() => {
    void loadHistory();
  }, [loadHistory]);

  useEffect(() => {
    const stop = onRewindOpen(() => {
      if (entries === null) void loadHistory();
      input.current?.focus();
      input.current?.select();
    });
    return () => {
      void stop.then((unlisten) => unlisten());
    };
  }, [entries, loadHistory]);

  useEffect(() => {
    const element = panel.current;
    if (!element) return;
    const observer = new ResizeObserver(() => {
      const content = [...element.children].reduce(
        (sum, child) => sum + child.scrollHeight,
        0,
      );
      void api.rewindFit(content);
    });
    observer.observe(element);
    for (const child of element.querySelectorAll("[data-log] > *, form")) {
      observer.observe(child);
    }
    return () => observer.disconnect();
  }, [entries]);

  useLayoutEffect(() => {
    const element = log.current;
    if (!element || keepBottom.current === null) return;
    element.scrollTop =
      element.scrollHeight - element.clientHeight - keepBottom.current;
    keepBottom.current = null;
  }, [entries]);

  const loadOlder = async () => {
    const first = entries?.[0];
    const element = log.current;
    if (!older || !first || !element) return;
    setOlder(false);
    try {
      const turns = await api.rewindHistory(first.turn.id);
      keepBottom.current =
        element.scrollHeight - element.clientHeight - element.scrollTop;
      setEntries((current) => [
        ...turns.reverse().map((turn) => ({ turn, page: null })),
        ...(current ?? []),
      ]);
      setOlder(turns.length === HISTORY_PAGE);
    } catch (err) {
      setError(String(err));
    }
  };

  const ask = async () => {
    setAsking(true);
    try {
      const asked = await api.rewindAsk(query);
      keepBottom.current = 0;
      setEntries((current) => [...(current ?? []), asked]);
      setQuery("");
      setRecall(null);
      setError(null);
    } catch (err) {
      setError(String(err));
    } finally {
      setAsking(false);
    }
  };

  /** Finds a past question's sessions again, so retention and exclusions apply. */
  const showResults = async (id: number) => {
    const entry = entries?.find((candidate) => candidate.turn.id === id);
    if (!entry) return;
    try {
      const page = await api.rewindResults(entry.turn.question);
      setEntries((current) =>
        (current ?? []).map((candidate) =>
          candidate.turn.id === id ? { ...candidate, page } : candidate,
        ),
      );
    } catch (err) {
      setError(String(err));
    }
  };

  const questions = (entries ?? [])
    .map((entry) => entry.turn.question)
    .reverse();

  const onKey = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Escape") {
      e.preventDefault();
      void api.rewindHide();
    } else if (e.key === "ArrowUp" && (query === "" || recall !== null)) {
      e.preventDefault();
      const next = Math.min((recall ?? -1) + 1, questions.length - 1);
      if (next < 0) return;
      setRecall(next);
      setQuery(questions[next]);
    } else if (e.key === "ArrowDown" && recall !== null) {
      e.preventDefault();
      const next = recall - 1;
      setRecall(next < 0 ? null : next);
      setQuery(next < 0 ? "" : questions[next]);
    }
  };

  return (
    <div className="flex h-full flex-col" ref={panel}>
      <div
        data-log
        className="min-h-0 flex-1 divide-y overflow-y-auto px-4"
        ref={log}
        onScroll={(e) => {
          if (e.currentTarget.scrollTop === 0) void loadOlder();
        }}
      >
        {entries?.map(({ turn, page }) => (
          <section key={turn.id} className="flex flex-col gap-1.5 py-3">
            <div className="flex items-baseline justify-between gap-3">
              <span className="text-[15px] font-medium">{turn.question}</span>
              <span className="shrink-0 text-xs text-muted-foreground tabular-nums">{dateTime(turn.ts)}</span>
            </div>
            {turn.answer && <p className="text-sm">{turn.answer}</p>}
            {page === null ? (
              <Button
                type="button"
                variant="link"
                size="xs"
                className="self-start px-0 text-muted-foreground"
                onClick={() => void showResults(turn.id)}
              >
                {t("showResults")}
              </Button>
            ) : (
              <>
                {page.sessions.length === 0 && <Muted>{t("searchEmpty")}</Muted>}
                <ul className="flex flex-col gap-1.5 pt-1">
                  {page.sessions.map((session) => (
                    <li
                      key={`${session.source}-${session.app}-${session.start}-${session.end}`}
                      className="flex flex-col gap-1 rounded-lg bg-foreground/5 px-3 py-2"
                    >
                      <div className="flex flex-wrap items-center gap-1.5">
                        <Badge variant="outline">{t(`source_${session.source}`)}</Badge>
                        <span className="text-sm font-medium">{session.app}</span>
                        <span className="text-xs text-muted-foreground tabular-nums">{dateTime(session.start)}</span>
                      </div>
                      {(session.title || session.url) && (
                        <div className="text-xs break-all text-muted-foreground">{session.title || session.url}</div>
                      )}
                      <ul className="flex flex-col gap-0.5 text-sm">
                        {session.lines.map((line, index) => (
                          <li key={`${line.ts}-${index}`}>{line.text}</li>
                        ))}
                      </ul>
                    </li>
                  ))}
                </ul>
              </>
            )}
          </section>
        ))}
        {asking && <Muted className="py-3">{t("searching")}</Muted>}
        {error && (
          <div className="py-3">
            <Failed>{error}</Failed>
          </div>
        )}
      </div>
      <form
        role="search"
        className={cn("flex items-center gap-3 px-4 py-3", entries?.length && "border-t")}
        onSubmit={(e) => {
          e.preventDefault();
          if (query.trim()) void ask();
        }}
      >
        <Search aria-hidden className="size-5 shrink-0 text-muted-foreground" />
        <input
          ref={input}
          type="search"
          value={query}
          placeholder={t("searchPlaceholder")}
          aria-label={t("searchPlaceholder")}
          autoFocus
          readOnly={asking}
          className="w-full bg-transparent py-1.5 text-[17px] outline-none placeholder:text-muted-foreground [&::-webkit-search-cancel-button]:hidden"
          onChange={(e) => {
            setQuery(e.target.value);
            setRecall(null);
          }}
          onKeyDown={onKey}
        />
      </form>
    </div>
  );
}
