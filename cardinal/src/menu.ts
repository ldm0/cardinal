import { getName } from '@tauri-apps/api/app';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { Menu, MenuItem, PredefinedMenuItem, Submenu } from '@tauri-apps/api/menu';

const MENU_IDS = {
  root: 'cardinal.menu.root',
  file: 'cardinal.menu.file',
  quit: 'cardinal.menu.quit',
  help: 'cardinal.menu.help',
  status: 'cardinal.menu.status',
} as const;

let menuInitPromise: Promise<void> | null = null;
let currentMenu: Menu | null = null;
let statusMenuItem: MenuItem | null = null;

export function initializeAppMenu(): Promise<void> {
  if (!isTauri()) {
    return Promise.resolve();
  }

  if (!menuInitPromise) {
    menuInitPromise = createAndAssignMenu().catch((error) => {
      console.error('Failed to initialize Cardinal app menu', error);
      currentMenu = null;
      statusMenuItem = null;
      menuInitPromise = null;
      throw error;
    });
  }

  return menuInitPromise;
}

export async function updateAppMenuStatus(label: string): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await initializeAppMenu();
  const item = await ensureStatusMenuItem();
  if (!item) {
    return;
  }

  try {
    await item.setText(label);
  } catch (error) {
    console.error('Failed to update Cardinal menu status label', error);
  }
}

async function createAndAssignMenu(): Promise<void> {
  const appName = await getName().catch(() => 'Cardinal');

  const menu = await Menu.new({
    items: [
      await Submenu.new({
        text: 'File',
        items: [
          await PredefinedMenuItem.new({ item: { About: null } }),
          await PredefinedMenuItem.new({ item: "Separator" }),
          await MenuItem.new({
            text: `Quit ${appName}`,
            action: () => {
              void requestAppExit();
            },
          })
        ],
      })
    ]
  });

  console.log(menu);

  await menu.setAsAppMenu();

  currentMenu = menu;
}

async function ensureStatusMenuItem(): Promise<MenuItem | null> {
  if (statusMenuItem) {
    return statusMenuItem;
  }

  if (!currentMenu) {
    return null;
  }

  try {
    const candidate = await currentMenu.get(MENU_IDS.status);
    if (candidate) {
      statusMenuItem = candidate as MenuItem;
    }
  } catch (error) {
    console.error('Failed to acquire Cardinal menu status item', error);
    return null;
  }

  return statusMenuItem;
}

async function requestAppExit(): Promise<void> {
  try {
    await invoke('request_app_exit');
  } catch (error) {
    console.error('Failed to quit Cardinal from app menu', error);
  }
}
