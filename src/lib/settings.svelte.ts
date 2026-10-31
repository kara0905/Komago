import { getSettings, updateSettings, type Settings } from "./api";

class SettingsStore {
  current = $state<Settings | null>(null);

  async load() {
    this.current = await getSettings();
    this.applyTheme();
  }

  // L'interface suit la valeur renvoyée par Rust : en cas d'échec, elle ne
  // montre pas un réglage qui n'a pas été enregistré.
  async update(patch: Partial<Settings>) {
    if (!this.current) return;
    this.current = await updateSettings({ ...$state.snapshot(this.current), ...patch });
    this.applyTheme();
  }

  private applyTheme() {
    const theme = this.current?.theme;
    try {
      localStorage.setItem("komago-theme", theme ?? "system");
    } catch {
      // Simple confort : Rust reste la source de vérité.
    }
    if (theme === "light" || theme === "dark") {
      document.documentElement.dataset.theme = theme;
    } else {
      delete document.documentElement.dataset.theme;
    }
  }
}

export const settings = new SettingsStore();
