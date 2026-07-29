import { ExternalLink, ShieldCheck } from "lucide-react";
import type { Locale, Translator } from "./i18n";

type SourcesViewProps = {
  locale: Locale;
  t: Translator;
  onOpenUrl: (url: string) => void;
  onOpenGuide: () => void;
};

type SourceKind = "official" | "local" | "estimate" | "runtime";

type SourceRow = {
  id:
    | "plan"
    | "quota"
    | "credits"
    | "lifetime"
    | "daily"
    | "today"
    | "cost"
    | "trace"
    | "extensions"
    | "status";
  kind: SourceKind;
  method: string;
  flags: Array<"direct" | "derived" | "delayed">;
};

const rows: SourceRow[] = [
  {
    id: "plan",
    kind: "official",
    method: "account/read",
    flags: ["direct"],
  },
  {
    id: "quota",
    kind: "official",
    method: "account/rateLimits/read",
    flags: ["direct"],
  },
  {
    id: "credits",
    kind: "official",
    method: "account/rateLimits/read",
    flags: ["direct"],
  },
  {
    id: "lifetime",
    kind: "official",
    method: "account/usage/read · summary",
    flags: ["direct"],
  },
  {
    id: "daily",
    kind: "official",
    method: "account/usage/read · dailyUsageBuckets",
    flags: ["direct", "delayed"],
  },
  {
    id: "today",
    kind: "local",
    method: "$CODEX_HOME/sessions/**/*.jsonl · token_count",
    flags: ["derived"],
  },
  {
    id: "cost",
    kind: "estimate",
    method: "session model + token_count + pricing snapshot",
    flags: ["derived"],
  },
  {
    id: "trace",
    kind: "local",
    method: "session structured events · Codex X-Ray rules",
    flags: ["derived"],
  },
  {
    id: "extensions",
    kind: "local",
    method: "persisted trace index · structured tool events",
    flags: ["derived"],
  },
  {
    id: "status",
    kind: "runtime",
    method: "task_started/task_complete + file mtime",
    flags: ["derived"],
  },
];

function sourceLabel(kind: SourceKind, t: Translator): string {
  if (kind === "official") return t("sources.official");
  if (kind === "local") return t("sources.local");
  if (kind === "estimate") return t("sources.estimate");
  return t("sources.runtime");
}

export default function SourcesView({
  locale,
  t,
  onOpenUrl,
  onOpenGuide,
}: SourcesViewProps) {
  return (
    <main className="workspace sources-workspace">
      <header className="topbar">
        <div>
          <p className="eyebrow">{t("sources.eyebrow")}</p>
          <h1>{t("sources.title")}</h1>
          <span className="header-note">{t("sources.subtitle")}</span>
        </div>
        <button className="source-guide-button" onClick={onOpenGuide}>
          <span>{t("sources.openFile")}</span>
          <ExternalLink aria-hidden="true" />
        </button>
      </header>

      <section className="sources-table" aria-label={t("sources.title")}>
        <div className="sources-table-head">
          <span>{t("sources.metric")}</span>
          <span>{t("sources.origin")}</span>
          <span>{t("sources.formula")}</span>
        </div>
        {rows.map((row) => (
          <article className="source-row" key={row.id}>
            <div className="source-row-metric">
              <strong>{t(`sources.rows.${row.id}.metric`)}</strong>
              <div>
                {row.flags.map((flag) => (
                  <i className={flag} key={flag}>
                    {t(`sources.${flag}`)}
                  </i>
                ))}
              </div>
            </div>
            <div className="source-row-origin">
              <span className={`source-kind ${row.kind}`}>
                {sourceLabel(row.kind, t)}
              </span>
              <code>{row.method}</code>
            </div>
            <p>{t(`sources.rows.${row.id}.formula`)}</p>
          </article>
        ))}
      </section>

      <section className="source-privacy">
        <ShieldCheck className="source-privacy-icon" aria-hidden="true" />
        <div>
          <h2>{t("sources.privacyTitle")}</h2>
          <p>{t("sources.privacyBody")}</p>
        </div>
      </section>

      <section className="source-links">
        <button
          onClick={() => onOpenUrl("https://learn.chatgpt.com/docs/app-server")}
        >
          <span>01</span>
          <strong>{t("sources.officialDoc")}</strong>
          <small>learn.chatgpt.com/docs/app-server</small>
          <ExternalLink aria-hidden="true" />
        </button>
        <button
          onClick={() =>
            onOpenUrl(
              "https://learn.chatgpt.com/docs/config-file/config-reference",
            )
          }
        >
          <span>02</span>
          <strong>{t("sources.envDoc")}</strong>
          <small>$CODEX_HOME · sqlite_home</small>
          <ExternalLink aria-hidden="true" />
        </button>
      </section>

      <footer>
        <span>Codex X-Ray data guide · {locale}</span>
        <span>{t("footer.privacy")}</span>
      </footer>
    </main>
  );
}
