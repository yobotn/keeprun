// UI language: catalogs, the reactive locale, text direction and number and date formatting.
import { en, type Catalog, type Key, type PluralForms } from "./en";
import ar from "./ar";
import de from "./de";
import es from "./es";
import fr from "./fr";
import he from "./he";
import it from "./it";
import ja from "./ja";
import ko from "./ko";
import nl from "./nl";
import pl from "./pl";
import pt from "./pt-BR";
import ru from "./ru";
import tr from "./tr";
import uk from "./uk";
import zh from "./zh-CN";

export type { Key };

export interface Language {
  code: string;
  /** Name in the language itself, as shown in the picker. */
  name: string;
  rtl?: boolean;
  catalog: Catalog | typeof en;
  /** Locale used for Intl formatting, when it differs from `code`. */
  intl?: string;
}

// Sorted by code. The picker shows native names.
export const languages: Language[] = [
  { code: "ar", name: "العربية", rtl: true, catalog: ar, intl: "ar-u-nu-latn" },
  { code: "de", name: "Deutsch", catalog: de },
  { code: "en", name: "English", catalog: en },
  { code: "es", name: "Español", catalog: es },
  { code: "fr", name: "Français", catalog: fr },
  { code: "he", name: "עברית", rtl: true, catalog: he },
  { code: "it", name: "Italiano", catalog: it },
  { code: "ja", name: "日本語", catalog: ja },
  { code: "ko", name: "한국어", catalog: ko },
  { code: "nl", name: "Nederlands", catalog: nl },
  { code: "pl", name: "Polski", catalog: pl },
  { code: "pt-BR", name: "Português (Brasil)", catalog: pt },
  { code: "ru", name: "Русский", catalog: ru },
  { code: "tr", name: "Türkçe", catalog: tr },
  { code: "uk", name: "Українська", catalog: uk },
  { code: "zh-CN", name: "简体中文", catalog: zh },
];

const KEY = "keeprun.locale";

/** Best match for the system languages, falling back to English. */
function systemLanguage(): Language {
  const wanted = navigator.languages?.length ? navigator.languages : [navigator.language];
  for (const tag of wanted) {
    const exact = languages.find((l) => l.code.toLowerCase() === tag.toLowerCase());
    if (exact) return exact;
    const base = tag.split("-")[0].toLowerCase();
    const loose = languages.find((l) => l.code.split("-")[0].toLowerCase() === base);
    if (loose) return loose;
  }
  return languages.find((l) => l.code === "en")!;
}

function readChoice(): string {
  try {
    const v = localStorage.getItem(KEY);
    if (v && (v === "system" || languages.some((l) => l.code === v))) return v;
  } catch {
    // Storage can be unavailable. Follow the system language.
  }
  return "system";
}

/** `choice` is "system" or a language code. `lang` is the language in use. */
export const locale = $state({ choice: readChoice(), lang: systemLanguage() });

export const systemLang = systemLanguage();

function resolve(choice: string): Language {
  return choice === "system" ? systemLang : (languages.find((l) => l.code === choice) ?? systemLang);
}

locale.lang = resolve(locale.choice);

export function setLocale(choice: string) {
  locale.choice = choice;
  locale.lang = resolve(choice);
  applyDocument();
  try {
    localStorage.setItem(KEY, choice);
  } catch {
    // Not persisted. The choice still applies for this session.
  }
}

/** Set lang and dir on <html>, which drive fonts (see app.css) and mirroring. */
export function applyDocument() {
  const el = document.documentElement;
  el.lang = locale.lang.code;
  el.dir = locale.lang.rtl ? "rtl" : "ltr";
}

export function intlLocale(): string {
  return locale.lang.intl ?? locale.lang.code;
}

type Params = Record<string, string | number>;

function fill(text: string, params?: Params): string {
  if (!params) return text;
  return text.replace(/\{(\w+)\}/g, (m, k) => (k in params ? String(params[k]) : m));
}

const pluralRules = new Map<string, Intl.PluralRules>();

function plural(forms: PluralForms, count: number): string {
  const loc = intlLocale();
  let rules = pluralRules.get(loc);
  if (!rules) pluralRules.set(loc, (rules = new Intl.PluralRules(loc)));
  return forms[rules.select(count)] ?? forms.other;
}

/** Translate `key`. Missing entries fall back to English. Plural entries use `params.count`. */
export function t(key: Key, params?: Params): string {
  const entry = ((locale.lang.catalog as Partial<Catalog>)[key] ?? en[key]) as string | PluralForms;
  const text = typeof entry === "string" ? entry : plural(entry, Number(params?.count ?? 0));
  return fill(text, params);
}
