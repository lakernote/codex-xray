import { getCurrentWindow } from "@tauri-apps/api/window";

export type Theme = "light" | "dark";

const THEME_STORAGE_KEY = "codex-xray.theme.v1";

export function readTheme(): Theme {
  try {
    const saved = window.localStorage.getItem(THEME_STORAGE_KEY);
    return saved === "dark" ? "dark" : "light";
  } catch {
    return "light";
  }
}

export function applyTheme(theme: Theme, syncNativeWindow = false) {
  document.documentElement.dataset.theme = theme;
  document.documentElement.style.colorScheme = theme;
  document
    .querySelector('meta[name="theme-color"]')
    ?.setAttribute("content", theme === "dark" ? "#101210" : "#f7f7f5");
  if (syncNativeWindow) {
    try {
      void getCurrentWindow().setTheme(theme).catch(() => {
        // The web theme still works when the native-window permission is absent.
      });
    } catch {
      // Browser-only Vite previews do not expose Tauri window metadata.
    }
  }
}

export function writeTheme(theme: Theme) {
  try {
    window.localStorage.setItem(THEME_STORAGE_KEY, theme);
  } catch {
    // A blocked preference store should not prevent theme switching.
  }
  applyTheme(theme, true);
}
