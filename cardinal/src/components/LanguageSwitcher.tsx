import React from 'react';
import { useTranslation } from 'react-i18next';
import { LANGUAGE_OPTIONS } from '../i18n/config';

type LanguageSwitcherProps = {
  className?: string;
};

const LanguageSwitcher = ({ className }: LanguageSwitcherProps): React.JSX.Element => {
  const { t, i18n } = useTranslation();

  const handleChange = (event: React.ChangeEvent<HTMLSelectElement>) => {
    const nextLang = event.target.value;
    void i18n.changeLanguage(nextLang);
  };

  const currentCode =
    LANGUAGE_OPTIONS.find((option) => i18n.language.startsWith(option.code))?.code ??
    LANGUAGE_OPTIONS[0].code;

  return (
    <label className={`language-switcher ${className ?? ''}`}>
      <span className="sr-only">{t('language.label')}</span>
      <span className="language-switcher__display" aria-hidden="true">
        {t('language.trigger')}
      </span>
      <select
        className="language-switcher__select"
        value={currentCode}
        onChange={handleChange}
        aria-label={t('language.label')}
      >
        {LANGUAGE_OPTIONS.map((option) => (
          <option key={option.code} value={option.code}>
            {option.label}
          </option>
        ))}
      </select>
    </label>
  );
};

export default LanguageSwitcher;
