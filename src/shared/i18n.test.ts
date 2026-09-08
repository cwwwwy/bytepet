/** i18n lookup, fallback and interpolation behaviour. */

import { beforeEach, describe, expect, it } from "vitest";
import {
  DEFAULT_LANGUAGE,
  dictionaryKeys,
  getLanguage,
  hasTranslation,
  normalizeLanguage,
  resolveKey,
  setLanguage,
  t,
} from "./i18n";

describe("normalizeLanguage", () => {
  it("maps locale variants onto supported languages", () => {
    expect(normalizeLanguage("en")).toBe("en");
    expect(normalizeLanguage("en-US")).toBe("en");
    expect(normalizeLanguage("zh-CN")).toBe("zh-CN");
    expect(normalizeLanguage("zh-Hans-CN")).toBe("zh-CN");
    expect(normalizeLanguage("ZH_cn")).toBe("zh-CN");
  });

  it("falls back to the default language for unknown/empty input", () => {
    expect(normalizeLanguage("fr")).toBe(DEFAULT_LANGUAGE);
    expect(normalizeLanguage("")).toBe(DEFAULT_LANGUAGE);
    expect(normalizeLanguage(null)).toBe(DEFAULT_LANGUAGE);
    expect(normalizeLanguage(undefined)).toBe(DEFAULT_LANGUAGE);
  });
});

describe("t", () => {
  beforeEach(() => setLanguage("zh-CN"));

  it("returns the active language string", () => {
    expect(t("common.save")).toBe("保存");
    setLanguage("en");
    expect(t("common.save")).toBe("Save");
    expect(getLanguage()).toBe("en");
  });

  it("interpolates named variables", () => {
    setLanguage("en");
    expect(t("pet.frameSpec", { w: 24, h: 32, cols: 8, rows: 4 })).toBe(
      "24×32 · 8 cols × 4 rows",
    );
    expect(t("chat.messageCount", { n: 3 })).toBe("3 messages");
  });

  it("leaves unknown placeholders intact", () => {
    expect(t("time.minutesAgo", { other: 1 })).toBe("{n} 分钟前");
  });

  it("falls back to zh-CN when a key is missing from the active language", () => {
    const dictionaries = {
      "zh-CN": { "only.zh": "中文", "shared.key": "共享" },
      en: { "shared.key": "shared" },
    };
    expect(resolveKey("shared.key", "en", dictionaries)).toBe("shared");
    expect(resolveKey("only.zh", "en", dictionaries)).toBe("中文");
    expect(resolveKey("missing.key", "en", dictionaries)).toBe("missing.key");
    // The shipped dictionaries are fully aligned, so the fallback only fires
    // for keys that have not been translated yet.
    setLanguage("en");
    expect(t("common.save")).toBe("Save");
  });

  it("returns the key itself for unknown keys", () => {
    expect(t("nope.nope")).toBe("nope.nope");
    expect(hasTranslation("nope.nope")).toBe(false);
  });

  it("keeps both dictionaries aligned on core keys", () => {
    const zh = dictionaryKeys("zh-CN");
    const en = dictionaryKeys("en");
    const missingInEnglish = zh.filter((key) => !en.includes(key));
    expect(missingInEnglish).toEqual([]);
    expect(zh.length).toBeGreaterThan(150);
  });
});
