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

function readableError(reason: unknown, zh: boolean): string {
  const message = reason instanceof Error ? reason.message : String(reason);
  const normalized = message.toLowerCase();
  const missingPlatform = message.match(
    /platform [`'"]?([^`'"]+)[`'"]? was not found/i,
  )?.[1];
  if (
    missingPlatform ||
    (normalized.includes("platform") &&
      normalized.includes("not found") &&
      normalized.includes("updater"))
  ) {
    const platform = missingPlatform ? ` (${missingPlatform})` : "";
    return zh
      ? `当前发布缺少这台电脑的升级包${platform}，请从发布页手动下载，或等待下一版本修复。`
      : `This release has no updater package for this computer${platform}. Download it from Releases or wait for the next version.`;
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
