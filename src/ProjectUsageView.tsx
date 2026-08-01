import {
  ArrowDown,
  ArrowUp,
  ArrowUpRight,
  ArrowUpDown,
  ChevronDown,
  ChevronRight,
  Folder,
  MessageSquare,
} from "lucide-react";
import { Fragment, useMemo, useState } from "react";
import {
  formatExactTokens,
  formatExactUsd,
  formatReadableTokens,
  formatReadableUsd,
} from "./format";
import type { Locale } from "./i18n";
import SearchField from "./SearchField";
import type {
  ProjectUsageConversation,
  ProjectUsageProject,
  ProjectUsageSnapshot,
  ProjectUsageTurn,
} from "./types";

type Props = {
  locale: Locale;
  snapshot: ProjectUsageSnapshot | null;
  loading: boolean;
  error: string | null;
  onRefresh: () => void;
  onOpenTrace: (sessionId: string, turnId?: string) => void;
  onLoadTurns: (sessionId: string) => Promise<void>;
};

type ProjectSortKey =
  | "recent"
  | "fresh"
  | "cache"
  | "output"
  | "tokens"
  | "cost";
type SortDirection = "asc" | "desc";
type ProjectSort = {
  key: ProjectSortKey;
  direction: SortDirection;
};

function copy(locale: Locale, zh: string, en: string): string {
  return locale === "zh-CN" ? zh : en;
}

function formatDate(locale: Locale, value: string | null): string {
  if (!value) return "—";
  return new Intl.DateTimeFormat(locale, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));
}

function freshInput(row: {
  input_tokens: number;
  cached_input_tokens: number;
}): number {
  return Math.max(row.input_tokens - row.cached_input_tokens, 0);
}

function modelLabel(models: string[]): string {
  if (models.length === 0) return "—";
  return models.length === 1 ? models[0] : `${models[0]} +${models.length - 1}`;
}

function tokenCell(value: number) {
  return (
    <span className="project-usage-number" title={`${formatExactTokens(value)} Token`}>
      {formatReadableTokens(value)}
    </span>
  );
}

function costCell(
  locale: Locale,
  cost: number,
  pricedTokens: number,
  totalTokens: number,
) {
  if (pricedTokens === 0) {
    return (
      <span
        className="project-usage-number muted"
        title={copy(locale, "当前模型没有可用单价", "No price is available for this model")}
      >
        —
      </span>
    );
  }
  const partial = pricedTokens < totalTokens;
  return (
    <span
      className="project-usage-number"
      title={`${formatExactUsd(cost)}${partial ? ` · ${copy(locale, "部分 Token 未定价", "Some tokens are unpriced")}` : ""}`}
    >
      {formatReadableUsd(cost)}
      {partial && <sup>*</sup>}
    </span>
  );
}

function conversationTitle(
  locale: Locale,
  conversation: ProjectUsageConversation,
): string {
  return (
    conversation.title?.trim() ||
    `${copy(locale, "对话", "Conversation")} ${conversation.id.slice(0, 8)}`
  );
}

function turnTitle(locale: Locale, turn: ProjectUsageTurn): string {
  return `${copy(locale, "Turn", "Turn")} ${turn.sequence}`;
}

export default function ProjectUsageView({
  locale,
  snapshot,
  loading,
  error,
  onRefresh,
  onOpenTrace,
  onLoadTurns,
}: Props) {
  const [query, setQuery] = useState("");
  const [expandedProjects, setExpandedProjects] = useState<Set<string>>(
    () => new Set(),
  );
  const [expandedConversations, setExpandedConversations] = useState<Set<string>>(
    () => new Set(),
  );
  const [loadingTurns, setLoadingTurns] = useState<Set<string>>(
    () => new Set(),
  );
  const [turnErrors, setTurnErrors] = useState<Map<string, string>>(
    () => new Map(),
  );
  const [projectSort, setProjectSort] = useState<ProjectSort>({
    key: "recent",
    direction: "desc",
  });
  const normalizedQuery = query.trim().toLocaleLowerCase(locale);

  const visibleProjects = useMemo(() => {
    const projects = snapshot?.projects ?? [];
    const visible = normalizedQuery
      ? projects
          .map((project) => {
            const projectMatches = `${project.name} ${project.path}`
              .toLocaleLowerCase(locale)
              .includes(normalizedQuery);
            const conversations = projectMatches
              ? project.conversations
              : project.conversations.filter((conversation) =>
                  `${conversation.title ?? ""} ${conversation.id} ${conversation.models.join(" ")}`
                    .toLocaleLowerCase(locale)
                    .includes(normalizedQuery),
                );
            return { ...project, conversations };
          })
          .filter((project) => project.conversations.length > 0)
      : [...projects];

    return visible.sort((left, right) => {
      let comparison = 0;
      if (projectSort.key === "recent") {
        comparison =
          (Date.parse(left.updated_at ?? "") || 0) -
          (Date.parse(right.updated_at ?? "") || 0);
      } else if (projectSort.key === "fresh") {
        comparison = freshInput(left) - freshInput(right);
      } else if (projectSort.key === "cache") {
        comparison = left.cached_input_tokens - right.cached_input_tokens;
      } else if (projectSort.key === "output") {
        comparison = left.output_tokens - right.output_tokens;
      } else if (projectSort.key === "tokens") {
        comparison = left.total_tokens - right.total_tokens;
      } else {
        comparison = left.cost_usd - right.cost_usd;
      }
      if (comparison !== 0) {
        return projectSort.direction === "asc" ? comparison : -comparison;
      }
      return (
        (Date.parse(right.updated_at ?? "") || 0) -
          (Date.parse(left.updated_at ?? "") || 0) ||
        left.name.localeCompare(right.name, locale)
      );
    });
  }, [locale, normalizedQuery, projectSort, snapshot]);

  const changeProjectSort = (key: ProjectSortKey) => {
    setProjectSort((current) => ({
      key,
      direction:
        current.key === key && current.direction === "desc" ? "asc" : "desc",
    }));
  };

  const sortHeader = (
    key: ProjectSortKey,
    zhLabel: string,
    enLabel: string,
    zhMeaning?: string,
    enMeaning?: string,
  ) => {
    const active = projectSort.key === key;
    const label = copy(locale, zhLabel, enLabel);
    const meaning = copy(locale, zhMeaning ?? zhLabel, enMeaning ?? enLabel);
    const nextDirection: SortDirection =
      active && projectSort.direction === "desc" ? "asc" : "desc";
    const directionLabel = copy(
      locale,
      nextDirection === "asc" ? "升序" : "降序",
      nextDirection === "asc" ? "ascending" : "descending",
    );
    const SortIcon = active
      ? projectSort.direction === "asc"
        ? ArrowUp
        : ArrowDown
      : ArrowUpDown;
    return (
      <button
        className="project-usage-sort-button"
        type="button"
        data-active={active}
        onClick={() => changeProjectSort(key)}
        title={copy(
          locale,
          `按${meaning}${directionLabel}排列项目`,
          `Sort projects by ${meaning} ${directionLabel}`,
        )}
        aria-label={copy(
          locale,
          `按${meaning}${directionLabel}排列项目`,
          `Sort projects by ${meaning} ${directionLabel}`,
        )}
      >
        <span>{label}</span>
        <SortIcon aria-hidden="true" />
      </button>
    );
  };

  const ariaSort = (
    key: ProjectSortKey,
  ): "ascending" | "descending" | "none" =>
    projectSort.key === key
      ? projectSort.direction === "asc"
        ? "ascending"
        : "descending"
      : "none";

  const toggleProject = (path: string) => {
    setExpandedProjects((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };
  const loadTurns = async (conversation: ProjectUsageConversation) => {
    if (loadingTurns.has(conversation.id)) return;
    setLoadingTurns((current) => new Set(current).add(conversation.id));
    setTurnErrors((current) => {
      const next = new Map(current);
      next.delete(conversation.id);
      return next;
    });
    try {
      await onLoadTurns(conversation.id);
    } catch (reason) {
      setTurnErrors((current) =>
        new Map(current).set(
          conversation.id,
          reason instanceof Error ? reason.message : String(reason),
        ),
      );
    } finally {
      setLoadingTurns((current) => {
        const next = new Set(current);
        next.delete(conversation.id);
        return next;
      });
    }
  };
  const toggleConversation = (conversation: ProjectUsageConversation) => {
    const opening = !expandedConversations.has(conversation.id);
    setExpandedConversations((current) => {
      const next = new Set(current);
      if (next.has(conversation.id)) next.delete(conversation.id);
      else next.add(conversation.id);
      return next;
    });
    if (opening && !conversation.turns_indexed) {
      void loadTurns(conversation);
    }
  };

  if (!snapshot && loading) {
    return (
      <section className="project-usage-state" role="status">
        <span className="cost-index-pulse" />
        <div>
          <strong>{copy(locale, "正在读取项目用量", "Loading project usage")}</strong>
          <small>
            {copy(
              locale,
              "正在合并本地用量与 Codex 对话目录",
              "Merging local usage with the Codex conversation catalog",
            )}
          </small>
        </div>
      </section>
    );
  }

  if (!snapshot) {
    return (
      <section className="project-usage-state" role="alert">
        <div>
          <strong>{copy(locale, "项目用量暂不可用", "Project usage is unavailable")}</strong>
          <small>{error ?? copy(locale, "请刷新后重试", "Refresh to try again")}</small>
        </div>
        <button onClick={onRefresh}>{copy(locale, "重新读取", "Retry")}</button>
      </section>
    );
  }

  return (
    <section className="project-usage-section">
      <header className="project-usage-header">
        <div>
          <h2>{copy(locale, "按项目与对话", "By project and conversation")}</h2>
          <span>
            {copy(
              locale,
              "本地含 Token 的 Session，按原始工作目录归组；数量可能与 Codex 对话目录不同",
              "Local token-bearing sessions grouped by original working directory; counts can differ from the Codex conversation catalog",
            )}
          </span>
        </div>
        <div className="project-usage-tools">
          <SearchField
            className="project-usage-search"
            value={query}
            onChange={setQuery}
            placeholder={copy(
              locale,
              "搜索项目或对话",
              "Search project or conversation",
            )}
            ariaLabel={copy(
              locale,
              "搜索项目或对话",
              "Search projects or conversations",
            )}
            clearLabel={copy(locale, "清除搜索", "Clear search")}
          />
        </div>
      </header>

      <div className="project-usage-summary">
        <div>
          <span>{copy(locale, "项目", "Projects")}</span>
          <strong>{new Intl.NumberFormat(locale).format(snapshot.projects.length)}</strong>
        </div>
        <div>
          <span>{copy(locale, "对话", "Conversations")}</span>
          <strong>{new Intl.NumberFormat(locale).format(snapshot.sessions)}</strong>
        </div>
        <div>
          <span>
            {snapshot.turn_sessions_indexed < snapshot.sessions
              ? copy(locale, "已读取 Turn", "Loaded turns")
              : "Turn"}
          </span>
          <strong
            title={copy(
              locale,
              `${snapshot.turn_sessions_indexed}/${snapshot.sessions} 个对话已读取 Turn`,
              `Turn details loaded for ${snapshot.turn_sessions_indexed}/${snapshot.sessions} conversations`,
            )}
          >
            {new Intl.NumberFormat(locale).format(snapshot.turns)}
          </strong>
        </div>
        <div className="primary">
          <span>{copy(locale, "总 Token", "Total tokens")}</span>
          <strong title={`${formatExactTokens(snapshot.total_tokens)} Token`}>
            {formatReadableTokens(snapshot.total_tokens)}
          </strong>
        </div>
        <div>
          <span>{copy(locale, "API 等价成本", "API-equivalent cost")}</span>
          <strong title={formatExactUsd(snapshot.cost_usd)}>
            {formatReadableUsd(snapshot.cost_usd)}
          </strong>
        </div>
      </div>

      <div className="project-usage-table-wrap">
        <table className="project-usage-table">
          <caption>
            {copy(
              locale,
              "按项目、对话和 Turn 汇总的本地 Token 与 API 等价成本",
              "Local tokens and API-equivalent cost grouped by project, conversation, and turn",
            )}
          </caption>
          <thead>
            <tr>
              <th aria-sort={ariaSort("recent")}>
                {sortHeader(
                  "recent",
                  "项目 / 对话 / Turn",
                  "Project / conversation / turn",
                  "最近活跃时间",
                  "recent activity",
                )}
              </th>
              <th aria-sort={ariaSort("fresh")}>
                {sortHeader("fresh", "未缓存输入", "Fresh input")}
              </th>
              <th aria-sort={ariaSort("cache")}>
                {sortHeader("cache", "缓存读取", "Cache read")}
              </th>
              <th aria-sort={ariaSort("output")}>
                {sortHeader("output", "输出", "Output")}
              </th>
              <th aria-sort={ariaSort("tokens")}>
                {sortHeader("tokens", "总 Token", "Total tokens")}
              </th>
              <th aria-sort={ariaSort("cost")}>
                {sortHeader("cost", "估算成本", "Est. cost")}
              </th>
            </tr>
          </thead>
          <tbody>
            {visibleProjects.map((project) => {
              const projectKey = project.path || "__unassigned__";
              const projectOpen =
                expandedProjects.has(projectKey) ||
                (normalizedQuery.length > 0 &&
                  project.conversations.length <= 50);
              return (
                <Fragment key={projectKey}>
                  <ProjectRow
                    locale={locale}
                    project={project}
                    open={projectOpen}
                    onToggle={() => toggleProject(projectKey)}
                  />
                  {projectOpen &&
                    project.conversations.map((conversation) => {
                      const conversationOpen = expandedConversations.has(
                        conversation.id,
                      );
                      return (
                        <Fragment key={conversation.id}>
                          <ConversationRow
                            locale={locale}
                            conversation={conversation}
                            open={conversationOpen}
                            onToggle={() =>
                              toggleConversation(conversation)
                            }
                            onOpenTrace={() =>
                              onOpenTrace(conversation.id)
                            }
                          />
                          {conversationOpen &&
                            loadingTurns.has(conversation.id) && (
                              <tr className="project-usage-turn-state">
                                <td colSpan={6}>
                                  <span className="cost-index-pulse" />
                                  {copy(
                                    locale,
                                    "正在读取这个 Session 的 Turn",
                                    "Loading turns from this session",
                                  )}
                                </td>
                              </tr>
                            )}
                          {conversationOpen &&
                            !loadingTurns.has(conversation.id) &&
                            turnErrors.has(conversation.id) && (
                              <tr className="project-usage-turn-state error">
                                <td colSpan={6}>
                                  <span>{turnErrors.get(conversation.id)}</span>
                                  <button
                                    onClick={() => void loadTurns(conversation)}
                                  >
                                    {copy(locale, "重试", "Retry")}
                                  </button>
                                </td>
                              </tr>
                            )}
                          {conversationOpen &&
                            !loadingTurns.has(conversation.id) &&
                            !turnErrors.has(conversation.id) &&
                            conversation.turn_rows.map((turn) => (
                              <TurnRow
                                key={turn.id}
                                locale={locale}
                                turn={turn}
                                onOpenTrace={() =>
                                  onOpenTrace(conversation.id, turn.id)
                                }
                              />
                            ))}
                        </Fragment>
                      );
                    })}
                </Fragment>
              );
            })}
          </tbody>
        </table>
      </div>

      {visibleProjects.length === 0 && (
        <div className="project-usage-empty">
          {copy(locale, "没有匹配的项目或对话", "No matching project or conversation")}
        </div>
      )}

      <footer className="project-usage-footnote">
        <span>
          {copy(locale, "索引", "Indexed")}{" "}
          {new Intl.NumberFormat(locale).format(snapshot.files_indexed)}{" "}
          {copy(locale, "个 Session 文件", "session files")}
        </span>
        <span>
          {copy(locale, "更新于", "Updated")} {formatDate(locale, snapshot.generated_at)}
          {loading && ` · ${copy(locale, "正在刷新", "Refreshing")}`}
        </span>
        {error && <span className="error">{error}</span>}
      </footer>
    </section>
  );
}

function ProjectRow({
  locale,
  project,
  open,
  onToggle,
}: {
  locale: Locale;
  project: ProjectUsageProject;
  open: boolean;
  onToggle: () => void;
}) {
  return (
    <tr className="project-usage-row project">
      <td>
        <button
          className="project-usage-name"
          onClick={onToggle}
          aria-expanded={open}
        >
          {open ? <ChevronDown aria-hidden="true" /> : <ChevronRight aria-hidden="true" />}
          <Folder aria-hidden="true" />
          <span>
            <strong>{project.name || copy(locale, "未归属目录", "Unassigned")}</strong>
            <small title={project.path}>
              {project.sessions} {copy(locale, "个对话", "conversations")} ·{" "}
              {project.turn_sessions_indexed < project.sessions
                ? `${project.turns} ${copy(locale, "个已读取 Turn", "loaded turns")}`
                : `${project.turns} Turn`}
              {project.updated_at ? ` · ${formatDate(locale, project.updated_at)}` : ""}
            </small>
          </span>
        </button>
      </td>
      <td>{tokenCell(freshInput(project))}</td>
      <td>{tokenCell(project.cached_input_tokens)}</td>
      <td>{tokenCell(project.output_tokens)}</td>
      <td>{tokenCell(project.total_tokens)}</td>
      <td>
        {costCell(
          locale,
          project.cost_usd,
          project.priced_tokens,
          project.total_tokens,
        )}
      </td>
    </tr>
  );
}

function ConversationRow({
  locale,
  conversation,
  open,
  onToggle,
  onOpenTrace,
}: {
  locale: Locale;
  conversation: ProjectUsageConversation;
  open: boolean;
  onToggle: () => void;
  onOpenTrace: () => void;
}) {
  return (
    <tr className="project-usage-row conversation">
      <td>
        <div className="project-usage-conversation-cell">
          <button
            className="project-usage-name"
            onClick={onToggle}
            aria-expanded={open}
          >
            {open ? <ChevronDown aria-hidden="true" /> : <ChevronRight aria-hidden="true" />}
            <MessageSquare aria-hidden="true" />
            <span>
              <strong>{conversationTitle(locale, conversation)}</strong>
              <small>
                {conversation.turns_indexed
                  ? `${conversation.turns} Turn`
                  : copy(locale, "展开读取 Turn", "Expand to load turns")}{" "}
                · {modelLabel(conversation.models)}
                {conversation.is_subagent
                  ? ` · ${copy(locale, "子 Agent", "Sub-agent")}`
                  : ""}
                {conversation.updated_at
                  ? ` · ${formatDate(locale, conversation.updated_at)}`
                  : ""}
              </small>
            </span>
          </button>
          <button
            className="project-usage-trace-link"
            onClick={onOpenTrace}
            title={copy(locale, "打开执行追踪", "Open execution trace")}
            aria-label={copy(locale, "打开执行追踪", "Open execution trace")}
          >
            <ArrowUpRight aria-hidden="true" />
          </button>
        </div>
      </td>
      <td>{tokenCell(freshInput(conversation))}</td>
      <td>{tokenCell(conversation.cached_input_tokens)}</td>
      <td>{tokenCell(conversation.output_tokens)}</td>
      <td>{tokenCell(conversation.total_tokens)}</td>
      <td>
        {costCell(
          locale,
          conversation.cost_usd,
          conversation.priced_tokens,
          conversation.total_tokens,
        )}
      </td>
    </tr>
  );
}

function TurnRow({
  locale,
  turn,
  onOpenTrace,
}: {
  locale: Locale;
  turn: ProjectUsageTurn;
  onOpenTrace: () => void;
}) {
  return (
    <tr className="project-usage-row turn">
      <td>
        <button
          className="project-usage-name"
          onClick={onOpenTrace}
          title={copy(locale, "在执行追踪中打开", "Open in execution trace")}
        >
          <span className="turn-branch" aria-hidden="true" />
          <span>
            <strong>{turnTitle(locale, turn)}</strong>
            <small>
              {modelLabel(turn.models)}
              {turn.started_at ? ` · ${formatDate(locale, turn.started_at)}` : ""}
            </small>
          </span>
          <ArrowUpRight className="turn-open-icon" aria-hidden="true" />
        </button>
      </td>
      <td>{tokenCell(freshInput(turn))}</td>
      <td>{tokenCell(turn.cached_input_tokens)}</td>
      <td>{tokenCell(turn.output_tokens)}</td>
      <td>{tokenCell(turn.total_tokens)}</td>
      <td>
        {costCell(locale, turn.cost_usd, turn.priced_tokens, turn.total_tokens)}
      </td>
    </tr>
  );
}
