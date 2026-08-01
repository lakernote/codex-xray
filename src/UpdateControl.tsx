import { getVersion } from "@tauri-apps/api/app";
import { relaunch } from "@tauri-apps/plugin-process";
import {
  check as checkForUpdate,
  type DownloadEvent,
  type Update,
} from "@tauri-apps/plugin-updater";
import {
  Download,
  ExternalLink,
  LoaderCircle,
  RefreshCw,
  X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { localDateKey } from "./format";
import type { Locale } from "./i18n";

const RELEASES_PAGE = "https://github.com/lakernote/codex-xray/releases";
const LATEST_RELEASE_API =
  "https://api.github.com/repos/lakernote/codex-xray/releases/latest";
const LAST_CHECK_KEY = "codex-xray.update-last-check.v2";
const IGNORED_VERSION_KEY = "codex-xray.update-ignored-version.v2";

type ReleaseSnapshot = {
  currentVersion: string;
  version: string;
  notes: string | null;
  publishedAt: string | null;
};

type UpdatePhase =
  | "idle"
  | "checking"
  | "current"
  | "available"
  | "downloading"
  | "installing"
  | "error";

type Props = {
  locale: Locale;
  onOpenUrl: (url: string) => void;
};

type GithubRelease = {
  tag_name?: string;
};

function compareVersionIdentifiers(left: string, right: string): number {
  const leftNumeric = /^\d+$/.test(left);
  const rightNumeric = /^\d+$/.test(right);
  if (leftNumeric && rightNumeric) return Number(left) - Number(right);
  if (leftNumeric) return -1;
  if (rightNumeric) return 1;
  return left.localeCompare(right);
}

function compareVersions(left: string, right: string): number {
  const parse = (value: string) => {
    const [core, prerelease = ""] = value.replace(/^v/i, "").split("-", 2);
    return {
      core: core.split(".").map((part) => Number(part) || 0),
      prerelease: prerelease ? prerelease.split(".") : [],
    };
  };
  const leftVersion = parse(left);
  const rightVersion = parse(right);
  const coreLength = Math.max(
    leftVersion.core.length,
    rightVersion.core.length,
  );
  for (let index = 0; index < coreLength; index += 1) {
    const difference =
      (leftVersion.core[index] ?? 0) - (rightVersion.core[index] ?? 0);
    if (difference !== 0) return difference;
  }
  if (leftVersion.prerelease.length === 0) {
    return rightVersion.prerelease.length === 0 ? 0 : 1;
  }
  if (rightVersion.prerelease.length === 0) return -1;
  const prereleaseLength = Math.max(
    leftVersion.prerelease.length,
    rightVersion.prerelease.length,
  );
  for (let index = 0; index < prereleaseLength; index += 1) {
    const leftIdentifier = leftVersion.prerelease[index];
    const rightIdentifier = rightVersion.prerelease[index];
    if (leftIdentifier == null) return -1;
    if (rightIdentifier == null) return 1;
    const difference = compareVersionIdentifiers(
      leftIdentifier,
      rightIdentifier,
    );
    if (difference !== 0) return difference;
  }
  return 0;
}

async function latestStableVersion(): Promise<string | null> {
  const response = await fetch(LATEST_RELEASE_API, {
    headers: { Accept: "application/vnd.github+json" },
    signal: AbortSignal.timeout(8_000),
  });
  if (!response.ok) return null;
  const release = (await response.json()) as GithubRelease;
  const version = release.tag_name?.trim().replace(/^v/i, "");
  return version || null;
}

function readableError(reason: unknown, zh: boolean): string {
  const message = reason instanceof Error ? reason.message : String(reason);
  const normalized = message.toLowerCase();
  const missingPlatform = message.match(
    /platform [`'"]?([^`'"]+)[`'"]? was not found/i,
  )?.[1];
  const missingFallbacks = message.match(
    /fallback platforms\s+[`'"]?(\[[^\]]+\])/i,
  )?.[1];
  if (
    missingPlatform ||
    missingFallbacks ||
    (normalized.includes("platform") &&
      (normalized.includes("not found") || normalized.includes("were found")) &&
      normalized.includes("platforms"))
  ) {
    const platform = missingPlatform ? ` (${missingPlatform})` : "";
    return zh
      ? `当前发布缺少这台电脑的应用内升级包${platform}。安装包仍可在发布页手动下载；发布流程会阻止后续版本再次漏包。`
      : `This release is missing the in-app updater package for this computer${platform}. The installer is still available on Releases, and future releases are blocked if an updater package is missing.`;
  }
  if (message.includes("404")) {
    return zh
      ? "更新服务尚未就绪，请稍后重试。"
      : "The update service is not ready yet. Try again later.";
  }
  if (
    normalized.includes("timed out") ||
    normalized.includes("timeout") ||
    normalized.includes("network") ||
    normalized.includes("failed to fetch")
  ) {
    return zh
      ? "无法连接 GitHub 更新服务，请检查网络后重试。"
      : "Could not reach the GitHub update service. Check your connection and try again.";
  }
  return message;
}

function releaseNotes(value: string | undefined): string | null {
  const normalized = value?.trim();
  if (!normalized) return null;
  return normalized.length > 700
    ? `${normalized.slice(0, 697).trimEnd()}…`
    : normalized;
}

export default function UpdateControl({ locale, onOpenUrl }: Props) {
  const zh = locale === "zh-CN";
  const updateRef = useRef<Update | null>(null);
  const downloadedBytesRef = useRef(0);
  const downloadTotalRef = useRef<number | null>(null);
  const [currentVersion, setCurrentVersion] = useState("");
  const [snapshot, setSnapshot] = useState<ReleaseSnapshot | null>(null);
  const [phase, setPhase] = useState<UpdatePhase>("idle");
  const [dialogOpen, setDialogOpen] = useState(false);
  const [manualAttempt, setManualAttempt] = useState(false);
  const [downloadPercent, setDownloadPercent] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const closeUpdateResource = useCallback(() => {
    const update = updateRef.current;
    updateRef.current = null;
    if (update) void update.close().catch(() => undefined);
  }, []);

  const check = useCallback(
    async (manual: boolean, version: string) => {
      if (!version) return;
      if (manual) setManualAttempt(true);
      setPhase("checking");
      setError(null);
      closeUpdateResource();
      try {
        try {
          const latestVersion = await latestStableVersion();
          if (
            latestVersion &&
            compareVersions(latestVersion, version) <= 0
          ) {
            window.localStorage.setItem(LAST_CHECK_KEY, localDateKey());
            setSnapshot(null);
            setPhase("current");
            return;
          }
        } catch {
          // The signed updater endpoint remains the source of truth when the
          // GitHub version preflight is unavailable.
        }
        const update = await checkForUpdate({ timeout: 12_000 });
        window.localStorage.setItem(LAST_CHECK_KEY, localDateKey());
        if (!update) {
          setSnapshot(null);
          setPhase("current");
          return;
        }
        updateRef.current = update;
        const next: ReleaseSnapshot = {
          currentVersion: update.currentVersion,
          version: update.version,
          notes: releaseNotes(update.body),
          publishedAt: update.date ?? null,
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
          setError(readableError(reason, zh));
          setPhase("error");
        } else {
          setPhase("idle");
        }
      }
    },
    [closeUpdateResource, zh],
  );

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
      closeUpdateResource();
    };
  }, [check, closeUpdateResource]);

  const busy = phase === "downloading" || phase === "installing";

  useEffect(() => {
    if (!dialogOpen || busy) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setDialogOpen(false);
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [busy, dialogOpen]);

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
    closeUpdateResource();
    setPhase("idle");
    setDialogOpen(false);
  };

  const updateDownloadProgress = (event: DownloadEvent) => {
    if (event.event === "Started") {
      downloadedBytesRef.current = 0;
      downloadTotalRef.current = event.data.contentLength ?? null;
      setDownloadPercent(event.data.contentLength ? 0 : null);
      return;
    }
    if (event.event === "Progress") {
      downloadedBytesRef.current += event.data.chunkLength;
      const total = downloadTotalRef.current;
      if (total && total > 0) {
        setDownloadPercent(
          Math.min(100, Math.round((downloadedBytesRef.current / total) * 100)),
        );
      }
      return;
    }
    setDownloadPercent(100);
    setPhase("installing");
  };

  const install = async () => {
    const update = updateRef.current;
    if (!update) {
      setError(
        zh
          ? "升级信息已失效，请重新检查。"
          : "The update information expired. Check again.",
      );
      setPhase("error");
      return;
    }
    setError(null);
    setDownloadPercent(null);
    setPhase("downloading");
    try {
      await update.downloadAndInstall(updateDownloadProgress, {
        timeout: 5 * 60_000,
      });
      setPhase("installing");
      await relaunch();
    } catch (reason) {
      setError(readableError(reason, zh));
      setPhase("error");
    }
  };

  const channel = currentVersion.includes("-") ? "beta" : "stable";

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
            disabled={phase === "checking" || busy || !currentVersion}
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
            {zh ? `可升级到 v${snapshot.version}` : `Update to v${snapshot.version}`}
          </button>
        )}

        {phase === "current" && manualAttempt && (
          <small aria-live="polite">
            {zh ? "当前已是最新版本" : "You are up to date"}
          </small>
        )}

        {phase === "error" && error && (
          <small className="sidebar-update-error" role="alert">
            <strong>{zh ? "升级检查失败" : "Update check failed"}</strong>
            <span>{error}</span>
          </small>
        )}
      </section>

      {dialogOpen && snapshot && (
        <div
          className="update-notice-backdrop"
          role="presentation"
          onMouseDown={(event) => {
            if (!busy && event.currentTarget === event.target) {
              setDialogOpen(false);
            }
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
                <span>{zh ? "应用更新" : "App update"}</span>
                <h2 id="update-notice-title">
                  {zh ? "新版本可以安装" : "An update is ready"}
                </h2>
              </div>
              <button
                type="button"
                className="dialog-close"
                autoFocus
                disabled={busy}
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
                <small>{zh ? "新版本" : "Update"}</small>
                <strong>v{snapshot.version}</strong>
              </span>
            </div>

            <div className="update-notice-copy">
              {publishedAt && <small>{publishedAt}</small>}
              {snapshot.notes && <p>{snapshot.notes}</p>}
              {phase === "downloading" && (
                <div className="update-download-progress" role="status">
                  <span>
                    {zh ? "正在下载并校验" : "Downloading and verifying"}
                  </span>
                  <strong>
                    {downloadPercent == null ? "…" : `${downloadPercent}%`}
                  </strong>
                  <i>
                    <b
                      style={{
                        width:
                          downloadPercent == null ? "18%" : `${downloadPercent}%`,
                      }}
                    />
                  </i>
                </div>
              )}
              {phase === "installing" && (
                <span role="status">
                  {zh
                    ? "正在安装，完成后会自动重启。"
                    : "Installing. The app will restart when ready."}
                </span>
              )}
              {phase === "error" && error && (
                <span className="update-notice-error" role="alert">
                  {error}
                </span>
              )}
              {phase === "available" && (
                <span>
                  {zh
                    ? "升级包会先验证签名，再替换当前版本并重启。"
                    : "The package is signature-verified before the current version is replaced and restarted."}
                </span>
              )}
            </div>

            <footer>
              {!busy && phase !== "error" && (
                <button type="button" className="text-button" onClick={ignore}>
                  {zh ? "忽略此版本" : "Ignore this version"}
                </button>
              )}
              {phase === "error" && (
                <button
                  type="button"
                  className="text-button"
                  onClick={() => onOpenUrl(RELEASES_PAGE)}
                >
                  <ExternalLink aria-hidden="true" />
                  {zh ? "查看发布页" : "Open releases"}
                </button>
              )}
              <button
                type="button"
                className="primary-action"
                disabled={busy}
                onClick={() => void install()}
              >
                {busy ? (
                  <LoaderCircle className="spinning" aria-hidden="true" />
                ) : (
                  <Download aria-hidden="true" />
                )}
                {phase === "downloading"
                  ? zh
                    ? "正在下载"
                    : "Downloading"
                  : phase === "installing"
                    ? zh
                      ? "正在安装"
                      : "Installing"
                    : phase === "error"
                      ? zh
                        ? "重试安装"
                        : "Retry install"
                      : zh
                        ? "下载并安装"
                        : "Download and install"}
              </button>
            </footer>
          </section>
        </div>
      )}
    </>
  );
}
