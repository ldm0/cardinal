import React, { useCallback, useRef, useLayoutEffect, useState } from 'react';
import type { CSSProperties } from 'react';
import type { AppLifecycleStatus } from '../types/ipc';
import { useTranslation } from 'react-i18next';
import LanguageSwitcher from './LanguageSwitcher';

export type StatusTabKey = 'files' | 'events';

type StatusBarProps = {
  scannedFiles: number;
  processedEvents: number;
  lifecycleState: AppLifecycleStatus;
  searchDurationMs?: number | null;
  resultCount?: number | null;
  activeTab?: StatusTabKey;
  onTabChange?: (tab: StatusTabKey) => void;
  onRequestRescan?: () => void;
};

const TABS: StatusTabKey[] = ['files', 'events'];

const LIFECYCLE_META: Record<AppLifecycleStatus, { icon: string; tone: string }> = {
  Initializing: { icon: '○', tone: 'initializing' },
  Ready: { icon: '●', tone: 'ready' },
  Closing: { icon: '●', tone: 'closing' },
};

const StatusBar = ({
  scannedFiles,
  processedEvents,
  lifecycleState,
  searchDurationMs,
  resultCount,
  activeTab = 'files',
  onTabChange,
  onRequestRescan,
}: StatusBarProps): React.JSX.Element => {
  const { t } = useTranslation();
  const tabsRef = useRef<HTMLDivElement | null>(null);
  const filesTabRef = useRef<HTMLButtonElement | null>(null);
  const eventsTabRef = useRef<HTMLButtonElement | null>(null);
  const [sliderStyle, setSliderStyle] = useState<CSSProperties>({});

  useLayoutEffect(() => {
    // Keep the active-tab underline aligned even when labels resize or counters update.
    const updateSliderPosition = () => {
      const activeTabRef = activeTab === 'files' ? filesTabRef : eventsTabRef;
      if (activeTabRef.current && tabsRef.current) {
        const tabRect = activeTabRef.current.getBoundingClientRect();
        const containerRect = tabsRef.current.getBoundingClientRect();

        setSliderStyle({
          left: `${tabRect.left - containerRect.left}px`,
          width: `${tabRect.width}px`,
        });
      }
    };

    updateSliderPosition();
    // Re-align the slider when the viewport changes width.
    window.addEventListener('resize', updateSliderPosition);
    return () => window.removeEventListener('resize', updateSliderPosition);
  }, [activeTab, scannedFiles, processedEvents]);

  const handleSelect = useCallback(
    (tabKey: StatusTabKey) => {
      if (tabKey === activeTab) return;
      onTabChange?.(tabKey);
    },
    [activeTab, onTabChange],
  );

  const formattedResultCount =
    typeof resultCount === 'number' ? resultCount.toLocaleString() : null;
  const resultsText =
    typeof resultCount === 'number'
      ? t('statusBar.resultsCount', {
          count: resultCount,
          formatted: formattedResultCount ?? `${resultCount}`,
        })
      : t('statusBar.resultsUnavailable');
  const formattedDuration =
    typeof searchDurationMs === 'number' ? Math.round(searchDurationMs).toLocaleString() : null;
  const durationText =
    formattedDuration != null ? t('statusBar.duration', { value: formattedDuration }) : null;
  const searchDisplay = durationText
    ? t('statusBar.resultsWithDuration', { results: resultsText, duration: durationText })
    : resultsText;
  const lifecycleMeta = LIFECYCLE_META[lifecycleState] ?? LIFECYCLE_META.Initializing;
  const lifecycleLabel =
    t(`statusBar.lifecycle.${lifecycleState}`) ?? t('statusBar.lifecycle.Initializing');
  const rescanDisabled = lifecycleState !== 'Ready';
  const rescanTitle = rescanDisabled
    ? t('statusBar.rescan.disabledHint')
    : t('statusBar.rescan.enabledHint');
  const indicatorLabel = t('statusBar.aria.status', { status: lifecycleLabel });

  const handleRescanClick = useCallback(() => {
    if (rescanDisabled) {
      return;
    }
    onRequestRescan?.();
  }, [onRequestRescan, rescanDisabled]);

  return (
    <div className="status-bar">
      <div className="status-left">
        <div className="status-section">
          <span
            className={`readiness-indicator ${lifecycleMeta.tone}`}
            aria-label={indicatorLabel}
            title={indicatorLabel}
          >
            {lifecycleMeta.icon}
          </span>
          <span className="status-text">{lifecycleLabel}</span>
        </div>
        <div
          ref={tabsRef}
          className="status-tabs"
          role="tablist"
          aria-label={t('statusBar.aria.tablist')}
        >
          <div className="status-tabs-slider" style={sliderStyle} />
          {TABS.map((key) => {
            const isActive = activeTab === key;
            const label = t(`statusBar.tabs.${key}`);
            const value =
              key === 'files' ? scannedFiles.toLocaleString() : processedEvents.toLocaleString();
            const ref = key === 'files' ? filesTabRef : eventsTabRef;
            return (
              <button
                key={key}
                ref={ref}
                type="button"
                role="tab"
                aria-selected={isActive}
                className={`status-tab ${isActive ? 'is-active' : ''}`}
                data-tone={key}
                onClick={() => handleSelect(key)}
              >
                <span className="status-tab__label">{label}</span>
                <span className="status-tab__value">{value}</span>
              </button>
            );
          })}
        </div>
        <div className="status-controls">
          <button
            type="button"
            className="status-icon-button status-rescan-button"
            onClick={handleRescanClick}
            disabled={rescanDisabled}
            title={rescanTitle}
            aria-label={t('statusBar.aria.rescan')}
          >
            <span className="status-rescan-icon" aria-hidden="true">
              ↻
            </span>
            <span className="sr-only">{t('statusBar.aria.rescan')}</span>
          </button>
          <LanguageSwitcher className="status-icon-button status-language-switcher" />
        </div>
      </div>

      <div className="status-right">
        <div className="status-section">
          <span className="status-label">{t('statusBar.searchLabel')}</span>
          <span className="status-value" title={t('statusBar.resultsTitle')}>
            {searchDisplay}
          </span>
        </div>
      </div>
    </div>
  );
};

export default StatusBar;
