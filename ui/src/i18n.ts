import ptBr from "./locales/pt-BR.json";
import en from "./locales/en.json";

export type LocaleCode = "pt-br" | "en";

const catalogs: Record<LocaleCode, Record<string, string>> = {
  "pt-br": ptBr,
  en,
};

let currentLocale: LocaleCode = "pt-br";

export function setLocale(code: string): void {
  const normalized = code.toLowerCase().startsWith("en") ? "en" : "pt-br";
  currentLocale = normalized;
  document.documentElement.lang = normalized === "en" ? "en" : "pt-BR";
}

export function getLocale(): LocaleCode {
  return currentLocale;
}

export function t(key: string): string {
  return catalogs[currentLocale][key] ?? catalogs["pt-br"][key] ?? key;
}
