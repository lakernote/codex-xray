import { getVersion } from "@tauri-apps/api/app";
import { ExternalLink, RefreshCw, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { localDateKey } from "./format";
import type { Locale } from "./i18n";

const RELEASES_API =
  "https://api.github.com/repos/lakernote/codex-xray/releases?per_page=30";
const RELEASES_PAGE = "https://github.com/lakernote/codex-xray/releases";
const LAST_CHECK_KEY = "codex-xray.update-last-check.v1";
const IGNORED_VERSION_KEY = "codex-xray.update-ignored-version.v1";

type GitHubRelease = {
  tag_name: string;
  name: string | null;
  html_url: string;
  body: string | null;
  published_at: string | null;
  draft: boolean;
  prerelease: boolean;
};

type ReleaseSnapshot = {
  currentVersion: string;
  channel: "stable" | "beta";
  version: string;
  name: string;
  url: string;
  notes: string | null;
  publishedAt: string | null;
};

type ParsedVersion = {
  core: [number, number, number];
  prerelease: Array<number | string>;
};

type UpdatePhase = "idle" | "checking" | "current" | "available" | "error";

type Props = {
  locale: Locale;
  onOpenUrl: (url: string) => void;
};

function parseVersion(raw: string): ParsedVersion | null {
  const match = raw
    .trim()
    .replace(/^v/i, "")
    .match(/^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$/);
  if (!match) return null;
  const prerelease = match[4]
    ? match[4].split(".").map((part) => {
        const numeric = Number(part);
        return /^\d+$/.test(part) ? numeric : part.toLowerCase();
      })
    : [];
  return {
    core: [Number(match[1]), Number(match[2]), Number(match[3])],
    prerelease,
  };
}

function compareVersions(left: string, right: string): number {
  const a = parseVersion(left);
  const b = parseVersion(right);
  if (!a || !b) return 0;
  for (let index = 0; index < a.core.length; index += 1) {
    if (a.core[index] !== b.core[index]) {
      return a.core[index] > b.core[index] ? 1 : -1;
    }
  }
  if (a.prerelease.length === 0 && b.prerelease.length === 0) return 0;
  if (a.prerelease.length === 0) return 1;
  if (b.prerelease.length === 0) return -1;
  const length = Math.max(a.prerelease.length, b.prerelease.length);
  for (let index = 0; index < length; index += 1) {
    const leftPart = a.prerelease[index];
    const rightPart = b.prerelease[index];
    if (leftPart === undefined) return -1;
    if (rightPart === undefined) return 1;
    if (leftPart === rightPart) continue;
    if (typeof leftPart === "number" && typeof rightPart === "string") return -1;
    if (typeof leftPart === "string" && typeof rightPart === "number") return 1;
    return leftPart > rightPart ? 1 : -1;
  }
  return 0;
}

function latestRelease(
  releases: GitHubRelease[],
  currentVersion: string,
): GitHubRelease | null {
  const betaChannel = currentVersion.includes("-");
  return (
    releases
      .filter(
        (release) =>
          !release.draft &&
          (betaChannel || !release.prerelease) &&
          parseVersion(release.tag_name) != null &&
          compareVersions(release.tag_name, currentVersion) > 0,
      )
      .sort((left, right) => compareVersions(right.tag_name, left.tag_name))[0] ??
    null
  );
}

function readableError(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}

function releaseNotes(value: string | null): string | null {
  const normalized = value?.trim();
  if (!normalized) return null;
  return normalized.length > 700
    ? `${normalized.slice(0, 697).trimEnd()}…`
    : normalized;
}

export default function UpdateControl({ locale, onOpenUrl }: Props) {
  const zh = locale === "zh-CN";
  const [currentVersion, setCurrentVersion] = useState("");
  const [snapshot, setSnapshot] = useState<ReleaseSnapshot | null>(null);
  const [phase, setPhase] = useState<UpdatePhase>("idle");
  const [dialogOpen, setDialogOpen] = useState(false);
  const [manualAttempt, setManualAttempt] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const check = useCallback(async (manual: boolean, version: string) => {
    if (!version) return;
    if (manual) setManualAttempt(true);
    setPhase("checking");
    setError(null);
    const controller = new AbortController();
    const timeout = window.setTimeout(() => controller.abort(), 12_000);
    try {
      const response = await fetch(RELEASES_API, {
        headers: {
          Accept: "application/vnd.github+json",
        },
        cache: "no-store",
        signal: controller.signal,
      });
      if (!response.ok) {
        throw new Error(`GitHub API HTTP ${response.status}`);
      }
      const releases = (await response.json()) as GitHubRelease[];
      const release = latestRelease(releases, version);
      window.localStorage.setItem(LAST_CHECK_KEY, localDateKey());
      if (!release) {
        setSnapshot(null);
        setPhase("current");
        return;
      }
      const next: ReleaseSnapshot = {
        currentVersion: version,
        channel: version.includes("-") ? "beta" : "stable",
        version: release.tag_name.replace(/^v/i, ""),
        name: release.name?.trim() || release.tag_name,
        url: release.html_url || RELEASES_PAGE,
        notes: releaseNotes(release.body),
        publishedAt: release.published_at,
      };
      setSnapshot(next);
      const ignored = window.localStorage.getItem(IGNORED_VERSION_KEY);
      if (!manual && ignored === next.version) {
        setPhase("idle");
        return;
      }
      setPhase("available");
      setDialogOpen(true);
    } catch (reason) {
      if (manual) {
        setError(readableError(reason));
        setPhase("error");
      } else {
        setPhase("idle");
      }
    } finally {
      window.clearTimeout(timeout);
    }
  }, []);

  useEffect(() => {
    let disposed = false;
    let initialTimer: number | null = null;
    let dailyTimer: number | null = null;
    void getVersion()
      .then((version) => {
        if (disposed) return;
        setCurrentVersion(version);
        const checkIfDue = () => {
          if (
            !disposed &&
            window.localStorage.getItem(LAST_CHECK_KEY) !== localDateKey()
          ) {
            void check(false, version);
          }
        };
        initialTimer = window.setTimeout(checkIfDue, 2_500);
        dailyTimer = window.setInterval(checkIfDue, 60 * 60 * 1_000);
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      if (initialTimer != null) window.clearTimeout(initialTimer);
      if (dailyTimer != null) window.clearInterval(dailyTimer);
    };
  }, [check]);

  useEffect(() => {
    if (!dialogOpen) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setDialogOpen(false);
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [dialogOpen]);

  const channel = currentVersion.includes("-") ? "beta" : "stable";
  const publishedAt = useMemo(() => {
    if (!snapshot?.publishedAt) return null;
    return new Intl.DateTimeFormat(locale, {
      year: "numeric",
      month: "short",
      day: "numeric",
    }).format(new Date(snapshot.publishedAt));
  }, [locale, snapshot?.publishedAt]);

  const ignore = () => {
    if (snapshot) {
      window.localStorage.setItem(IGNORED_VERSION_KEY, snapshot.version);
    }
    setPhase("idle");
    setDialogOpen(false);
  };

  const openRelease = () => {
    onOpenUrl(snapshot?.url ?? RELEASES_PAGE);
    setDialogOpen(false);
  };

  return (
    <>
      <section
        className="sidebar-update"
        aria-label={zh ? "版本更新" : "Version updates"}
      >
        <div className="sidebar-update-heading">
          <span>
            {currentVersion ? `v${currentVersion}` : zh ? "版本" : "Version"}
            <small>{channel === "beta" ? "Beta" : "Stable"}</small>
          </span>
          <button
            type="button"
            onClick={() => void check(true, currentVersion)}
            disabled={phase === "checking" || !currentVersion}
            aria-label={zh ? "检查新版本" : "Check for a new version"}
            title={zh ? "检查新版本" : "Check for a new version"}
          >
            <RefreshCw
              className={phase === "checking" ? "spinning" : ""}
              aria-hidden="true"
            />
          </button>
        </div>

        {phase === "available" && snapshot && (
          <button
            type="button"
            className="sidebar-update-action"
            onClick={() => setDialogOpen(true)}
          >
            {zh ? `发现 v${snapshot.version}` : `v${snapshot.version} available`}
          </button>
        )}

        {phase === "current" && manualAttempt && (
          <small aria-live="polite">
            {zh ? "当前已是最新版本" : "You are up to date"}
          </small>
        )}

        {phase === "error" && error && (
          <small className="sidebar-update-error" role="alert" title={error}>
            {zh ? "检查失败，请稍后重试" : "Check failed. Try again later."}
          </small>
        )}
      </section>

      {dialogOpen && snapshot && (
        <div
          className="update-notice-backdrop"
          role="presentation"
          onMouseDown={(event) => {
            if (event.currentTarget === event.target) setDialogOpen(false);
          }}
        >
          <section
            className="update-notice"
            role="dialog"
            aria-modal="true"
            aria-labelledby="update-notice-title"
          >
            <header>
              <div>
                <span>{zh ? "版本更新" : "Version update"}</span>
                <h2 id="update-notice-title">
                  {zh ? "发现新版本" : "A new version is available"}
                </h2>
              </div>
              <button
                type="button"
                className="dialog-close"
                autoFocus
                aria-label={zh ? "稍后提醒" : "Remind me later"}
                onClick={() => setDialogOpen(false)}
              >
                <X aria-hidden="true" />
              </button>
            </header>

            <div className="update-notice-version">
              <span>
                <small>{zh ? "当前版本" : "Current"}</small>
                <strong>v{snapshot.currentVersion}</strong>
              </span>
              <span aria-hidden="true">→</span>
              <span>
                <small>{zh ? "最新版本" : "Latest"}</small>
                <strong>v{snapshot.version}</strong>
              </span>
            </div>

            <div className="update-notice-copy">
              <strong>{snapshot.name}</strong>
              {publishedAt && <small>{publishedAt}</small>}
              {snapshot.notes && <p>{snapshot.notes}</p>}
              <span>
                {zh
                  ? "应用只打开 GitHub Releases，不会自动下载或安装。"
                  : "The app only opens GitHub Releases. It never downloads or installs updates automatically."}
              </span>
            </div>

            <footer>
              <button type="button" className="text-button" onClick={ignore}>
                {zh ? "忽略此版本" : "Ignore this version"}
              </button>
              <button
                type="button"
                className="primary-action"
                onClick={openRelease}
              >
                <ExternalLink aria-hidden="true" />
                {zh ? "前往下载" : "Open downloads"}
              </button>
            </footer>
          </section>
        </div>
      )}
    </>
  );
}
