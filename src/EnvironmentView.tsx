import { invoke } from "@tauri-apps/api/core";
import {
  Check,
  Copy,
  FolderOpen,
  LoaderCircle,
  RefreshCw,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { Locale } from "./i18n";
import type { EnvironmentSnapshot } from "./types";

type Props = {
  locale: Locale;
};

type LocalPath = {
  label: string;
  path: string;
  detail?: string;
};

function copy(locale: Locale, zh: string, en: string): string {
  return locale === "zh-CN" ? zh : en;
}

export default function EnvironmentView({ locale }: Props) {
  const [snapshot, setSnapshot] = useState<EnvironmentSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [copiedPath, setCopiedPath] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setSnapshot(await invoke<EnvironmentSnapshot>("get_environment_snapshot"));
    } catch (cause) {
      setError(String(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const paths = useMemo<LocalPath[]>(() => {
    if (!snapshot) return [];
    return [
      {
        label: "CODEX_HOME",
        path: snapshot.codex_home,
        detail: copy(locale, "Codex 本地数据根目录", "Codex local data root"),
      },
      {
        label: copy(locale, "用户配置", "User config"),
        path: snapshot.config_path,
        detail: "config.toml",
      },
      {
        label: copy(locale, "会话记录", "Sessions"),
        path: snapshot.sessions_path,
        detail: copy(locale, "Session JSONL 文件", "Session JSONL files"),
      },
      {
        label: copy(locale, "X-Ray 数据", "X-Ray data"),
        path: snapshot.xray_data_path,
        detail: copy(locale, "索引、缓存和恢复点", "Index, cache, and restore points"),
      },
      {
        label: copy(locale, "分析数据库", "Analysis database"),
        path: snapshot.xray_sqlite_path,
        detail: "SQLite",
      },
      ...snapshot.extension_paths
        .filter((entry) => entry.exists)
        .map((entry) => ({
          label: entry.label,
          path: entry.path,
          detail:
            entry.item_count == null
              ? undefined
              : copy(
                  locale,
                  `${entry.item_count} 项`,
                  `${entry.item_count} item${entry.item_count === 1 ? "" : "s"}`,
                ),
        })),
    ];
  }, [locale, snapshot]);

  const copyPath = async (path: string) => {
    try {
      await navigator.clipboard.writeText(path);
      setCopiedPath(path);
      window.setTimeout(
        () => setCopiedPath((current) => (current === path ? null : current)),
        1400,
      );
    } catch (cause) {
      setError(
        copy(
          locale,
          `无法复制路径：${String(cause)}`,
          `Unable to copy path: ${String(cause)}`,
        ),
      );
    }
  };

  const revealPath = async (path: string) => {
    try {
      setError(null);
      await invoke("reveal_local_path", { path });
    } catch (cause) {
      setError(String(cause));
    }
  };

  if (!snapshot && loading) {
    return (
      <section className="diagnostic-paths">
        <div className="diagnostic-paths-state" aria-live="polite">
          <LoaderCircle className="standard-loader" aria-hidden="true" />
          <span>{copy(locale, "正在读取本地路径", "Loading local paths")}</span>
        </div>
      </section>
    );
  }

  if (!snapshot) {
    return (
      <section className="diagnostic-paths">
        <div className="diagnostic-paths-state" role="alert">
          <strong>
            {copy(locale, "无法读取本地路径", "Local paths unavailable")}
          </strong>
          <span>{error}</span>
          <button className="secondary-action" onClick={() => void load()}>
            {copy(locale, "重试", "Try again")}
          </button>
        </div>
      </section>
    );
  }

  return (
    <section className="diagnostic-paths">
      <header>
        <div>
          <h2>{copy(locale, "本地目录", "Local paths")}</h2>
          <span>
            {copy(
              locale,
              "打开所在位置，或复制完整路径",
              "Reveal a location or copy its full path",
            )}
          </span>
        </div>
        <button
          className={loading ? "refresh-button spinning" : "refresh-button"}
          onClick={() => void load()}
          disabled={loading}
          aria-label={copy(locale, "刷新路径", "Refresh paths")}
        >
          <RefreshCw aria-hidden="true" />
        </button>
      </header>

      <div className="diagnostic-path-list">
        {paths.map((entry) => (
          <div className="diagnostic-path-row" key={`${entry.label}-${entry.path}`}>
            <div>
              <strong>{entry.label}</strong>
              {entry.detail && <small>{entry.detail}</small>}
            </div>
            <code title={entry.path}>{entry.path}</code>
            <div className="diagnostic-path-actions">
              <button
                title={copy(locale, "打开所在位置", "Reveal in file manager")}
                aria-label={copy(
                  locale,
                  `打开 ${entry.label}`,
                  `Reveal ${entry.label}`,
                )}
                onClick={() => void revealPath(entry.path)}
              >
                <FolderOpen aria-hidden="true" />
              </button>
              <button
                title={copy(locale, "复制路径", "Copy path")}
                aria-label={copy(
                  locale,
                  `复制 ${entry.label} 路径`,
                  `Copy ${entry.label} path`,
                )}
                onClick={() => void copyPath(entry.path)}
              >
                {copiedPath === entry.path ? (
                  <Check aria-hidden="true" />
                ) : (
                  <Copy aria-hidden="true" />
                )}
              </button>
            </div>
          </div>
        ))}
      </div>

      {error && (
        <div className="inline-error" role="alert">
          {error}
        </div>
      )}
    </section>
  );
}
