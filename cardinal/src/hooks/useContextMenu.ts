import { useState, useCallback } from 'react';
import type { MouseEvent as ReactMouseEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ContextMenuItem } from '../components/ContextMenu';
import { useTranslation } from 'react-i18next';

type MenuType = 'file' | 'header' | null;

type ContextMenuState = {
  visible: boolean;
  x: number;
  y: number;
  type: MenuType;
  data: string | null;
};

type UseContextMenuResult = {
  menu: ContextMenuState;
  showContextMenu: (event: ReactMouseEvent<HTMLElement>, path: string) => void;
  showHeaderContextMenu: (event: ReactMouseEvent<HTMLElement>) => void;
  closeMenu: () => void;
  getMenuItems: () => ContextMenuItem[];
};

export function useContextMenu(autoFitColumns: (() => void) | null = null): UseContextMenuResult {
  const { t } = useTranslation();
  const [menu, setMenu] = useState<ContextMenuState>({
    visible: false,
    x: 0,
    y: 0,
    type: null,
    data: null,
  });

  // Centralised helper for toggling the active menu overlay.
  const showMenu = useCallback(
    (event: ReactMouseEvent<HTMLElement>, type: MenuType, data: string | null = null) => {
      event.preventDefault();
      event.stopPropagation();
      setMenu({
        visible: true,
        x: event.clientX,
        y: event.clientY,
        type,
        data,
      });
    },
    [],
  );

  const showContextMenu = useCallback(
    (event: ReactMouseEvent<HTMLElement>, path: string) => {
      showMenu(event, 'file', path);
    },
    [showMenu],
  );

  const showHeaderContextMenu = useCallback(
    (event: ReactMouseEvent<HTMLElement>) => {
      showMenu(event, 'header');
    },
    [showMenu],
  );

  const closeMenu = useCallback(() => {
    setMenu((prev) => ({ ...prev, visible: false }));
  }, []);

  const getMenuItems = useCallback((): ContextMenuItem[] => {
    const path = menu.data;
    if (menu.type === 'file' && path) {
      return [
        {
          label: t('contextMenu.openInFinder'),
          action: () => invoke('open_in_finder', { path }),
        },
        {
          label: t('contextMenu.copyPath'),
          action: () => navigator.clipboard.writeText(path),
        },
      ];
    }
    if (menu.type === 'header' && autoFitColumns) {
      return [
        {
          label: t('contextMenu.resetColumnWidths'),
          action: autoFitColumns,
        },
      ];
    }
    return [];
  }, [menu, autoFitColumns, t]);

  return {
    menu,
    showContextMenu,
    showHeaderContextMenu,
    closeMenu,
    getMenuItems,
  };
}
