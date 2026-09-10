/** Local view model for the shell (no router dependency). */

export type SettingsTab = "pet" | "persona" | "provider" | "memory" | "general" | "agent" | "about";

export type View = { kind: "chat" } | { kind: "settings"; tab: SettingsTab };

export const SETTINGS_TABS: SettingsTab[] = [
  "pet",
  "persona",
  "provider",
  "memory",
  "general",
  "agent",
  "about",
];

export function settingsTabLabelKey(tab: SettingsTab): string {
  return `settings.tab.${tab}`;
}
