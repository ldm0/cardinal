import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';
import en from './resources/en.json';
import zh from './resources/zh.json';
import ja from './resources/ja.json';
import fr from './resources/fr.json';
import es from './resources/es.json';
import de from './resources/de.json';

export type SupportedLanguage = 'en' | 'zh' | 'ja' | 'fr' | 'es' | 'de';

type LanguageOption = {
  code: SupportedLanguage;
  label: string;
};

export const LANGUAGE_OPTIONS: LanguageOption[] = [
  { code: 'en', label: 'English' },
  { code: 'zh', label: '中文' },
  { code: 'ja', label: '日本語' },
  { code: 'fr', label: 'Français' },
  { code: 'es', label: 'Español' },
  { code: 'de', label: 'Deutsch' },
];

const STORAGE_KEY = 'cardinal.language';
const DEFAULT_LANGUAGE: SupportedLanguage = 'en';

const resources = {
  en: { translation: en },
  zh: { translation: zh },
  ja: { translation: ja },
  fr: { translation: fr },
  es: { translation: es },
  de: { translation: de },
} as const;

const detectInitialLanguage = (): SupportedLanguage => {
  if (typeof window === 'undefined') {
    return DEFAULT_LANGUAGE;
  }

  try {
    const stored = window.localStorage.getItem(STORAGE_KEY) as SupportedLanguage | null;
    if (stored && resources[stored]) {
      return stored;
    }
  } catch (error) {
    console.warn('Unable to read saved language preference', error);
  }

  const browserLang = window.navigator.language?.split('-')?.[0] as SupportedLanguage | undefined;
  if (browserLang && resources[browserLang]) {
    return browserLang;
  }

  return DEFAULT_LANGUAGE;
};

void i18n.use(initReactI18next).init({
  resources,
  lng: detectInitialLanguage(),
  fallbackLng: DEFAULT_LANGUAGE,
  interpolation: {
    escapeValue: false,
  },
  returnNull: false,
});

if (typeof document !== 'undefined') {
  document.documentElement.lang = i18n.language;
}

i18n.on('languageChanged', (lng) => {
  if (typeof window !== 'undefined') {
    try {
      window.localStorage.setItem(STORAGE_KEY, lng);
    } catch (error) {
      console.warn('Unable to persist language preference', error);
    }
  }
  if (typeof document !== 'undefined') {
    document.documentElement.lang = lng;
  }
});

export { i18n as default, STORAGE_KEY as LANGUAGE_STORAGE_KEY };
