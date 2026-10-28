import { useState } from "react";
import { api, type SearchPage } from "./api";
import { dateTime } from "./format";
import { t } from "./i18n";

const PAGE = 30;

/** One box over screen text and call transcripts, results grouped by session. */
export default function Search() {
  const [query, setQuery] = useState("");
  const [asked, setAsked] = useState<string | null>(null);
  const [page, setPage] = useState<SearchPage | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async (text: string, offset: number) => {
    try {
      const found = await api.search(text, offset, PAGE);
      setPage((current) =>
        offset === 0 || !current
          ? found
          : { sessions: [...current.sessions, ...found.sessions], next_offset: found.next_offset },
      );
      setAsked(text);
      setError(null);
    } catch (err) {
      setError(String(err));
    }
  };

  return (
    <>
      <form
        className="inline"
        role="search"
        onSubmit={(e) => {
          e.preventDefault();
          if (query.trim()) void run(query.trim(), 0);
        }}
      >
        <input
          type="search"
          value={query}
          placeholder={t("searchPlaceholder")}
          aria-label={t("searchPlaceholder")}
          onChange={(e) => setQuery(e.target.value)}
        />
        <button type="submit" disabled={query.trim() === ""}>
          {t("searchButton")}
        </button>
      </form>
      <p className="muted">{t("searchHint")}</p>
      {error && <p role="alert">{error}</p>}
      {page && page.sessions.length === 0 && <p className="muted">{t("searchEmpty")}</p>}
      <ul className="sessions">
        {page?.sessions.map((session) => (
          <li key={`${session.source}-${session.app}-${session.start}-${session.end}`}>
            <div className="workflow-head">
              <span className="tag">{t(`source_${session.source}`)}</span>
              <strong>{session.app}</strong>
              <span className="muted">{dateTime(session.start)}</span>
            </div>
            {(session.title || session.url) && (
              <div className="muted session-where">{session.title || session.url}</div>
            )}
            <ul className="lines">
              {session.lines.map((line, index) => (
                <li key={`${line.ts}-${index}`}>{line.text}</li>
              ))}
            </ul>
          </li>
        ))}
      </ul>
      {asked && page?.next_offset != null && (
        <button
          type="button"
          className="secondary"
          onClick={() => void run(asked, page.next_offset ?? 0)}
        >
          {t("showMore")}
        </button>
      )}
    </>
  );
}
