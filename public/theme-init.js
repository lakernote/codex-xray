try {
  const savedTheme = localStorage.getItem("codex-xray.theme.v1");
  const theme = savedTheme === "dark" ? "dark" : "light";
  document.documentElement.dataset.theme = theme;
  document.documentElement.style.colorScheme = theme;
  document
    .querySelector('meta[name="theme-color"]')
    ?.setAttribute("content", theme === "dark" ? "#101210" : "#f7f7f5");
} catch {
  document.documentElement.dataset.theme = "light";
  document.documentElement.style.colorScheme = "light";
}
