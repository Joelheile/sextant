import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { api, onRewindOpen, type SearchPage, type Turn } from "./api";
import { dateTime } from "./format";
import { t } from "./i18n";

/** Matches `REWIND_PAGE` and `REWIND_RESULTS` in `app/src/screens.rs`. */
const HISTORY_PAGE = 20;
const RESULTS = 5;

type Entry = { turn: Turn; page: SearchPage | null };

/** The search panel: past questions scroll up like a chat, the box sits at the bottom. */
export default function Rewind() {
  const [entries, setEntries] = useState<Entry[] | null>(null);
  const [older, setOlder] = useState(false);
  const [query, setQuery] = useState("");
  /** How far back Up has gone through past questions, or null while typing. */
  const [recall, setRecall] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
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
    for (const child of element.querySelectorAll(".rewind-log > *, form")) {
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
    try {
      const asked = await api.rewindAsk(query);
      keepBottom.current = 0;
      setEntries((current) => [...(current ?? []), asked]);
      setQuery("");
      setRecall(null);
      setError(null);
    } catch (err) {
      setError(String(err));
    }
  };

  /** Loads results for a past question, or the next few for one already shown. */
  const showResults = async (id: number) => {
    const entry = entries?.find((candidate) => candidate.turn.id === id);
    if (!entry) return;
    try {
      const offset = entry.page?.next_offset ?? 0;
      const found = await api.search(entry.turn.question, offset, RESULTS);
      setEntries((current) =>
        (current ?? []).map((candidate) =>
          candidate.turn.id === id
            ? {
                ...candidate,
                page: {
                  sessions: [
                    ...(candidate.page?.sessions ?? []),
                    ...found.sessions,
                  ],
                  next_offset: found.next_offset,
                },
              }
            : candidate,
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
    <div className="rewind" ref={panel}>
      <div
        className="rewind-log"
        ref={log}
        onScroll={(e) => {
          if (e.currentTarget.scrollTop === 0) void loadOlder();
        }}
      >
        {entries?.map(({ turn, page }) => (
          <section key={turn.id} className="turn">
            <div className="turn-head">
              <strong>{turn.question}</strong>
              <span className="muted">{dateTime(turn.ts)}</span>
            </div>
            {turn.answer && <p>{turn.answer}</p>}
            {page === null ? (
              <button
                type="button"
                className="link"
                onClick={() => void showResults(turn.id)}
              >
                {t("showResults")}
              </button>
            ) : (
              <>
                {page.sessions.length === 0 && (
                  <p className="muted">{t("searchEmpty")}</p>
                )}
                <ul className="sessions">
                  {page.sessions.map((session) => (
                    <li
                      key={`${session.source}-${session.app}-${session.start}-${session.end}`}
                    >
                      <div className="workflow-head">
                        <span className="tag">
                          {t(`source_${session.source}`)}
                        </span>
                        <strong>{session.app}</strong>
                        <span className="muted">{dateTime(session.start)}</span>
                      </div>
                      {(session.title || session.url) && (
                        <div className="muted session-where">
                          {session.title || session.url}
                        </div>
                      )}
                      <ul className="lines">
                        {session.lines.map((line, index) => (
                          <li key={`${line.ts}-${index}`}>{line.text}</li>
                        ))}
                      </ul>
                    </li>
                  ))}
                </ul>
                {page.next_offset != null && (
                  <button
                    type="button"
                    className="link"
                    onClick={() => void showResults(turn.id)}
                  >
                    {t("showMore")}
                  </button>
                )}
              </>
            )}
          </section>
        ))}
        {error && <p role="alert">{error}</p>}
      </div>
      <form
        role="search"
        onSubmit={(e) => {
          e.preventDefault();
          if (query.trim()) void ask();
        }}
      >
        <input
          ref={input}
          type="search"
          value={query}
          placeholder={t("searchPlaceholder")}
          aria-label={t("searchPlaceholder")}
          autoFocus
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
