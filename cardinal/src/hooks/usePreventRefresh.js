import { useEffect } from 'react';

export const usePreventRefresh = () => {
  useEffect(() => {
    const handleKeyDown = (event) => {
      // Prevent F5 or Ctrl+R (Windows/Linux) and Command+R (Mac) from refreshing the page
      if (
        event.key === 'F5' ||
        (event.ctrlKey && event.key === 'r') ||
        (event.metaKey && event.key === 'r')
      ) {
        event.preventDefault();
      }
    };

    const handleContextMenu = (event) => {
      // Only prevent the default context menu if the click is not on an element
      // that should have a custom context menu.
      // A simple check could be to see if the target or its parents have a specific class or attribute.
      // For now, we will prevent it everywhere except on rows and headers.
      if (!event.target.closest('.virtual-list') && !event.target.closest('.column-header')) {
        event.preventDefault();
      }
    };

    document.addEventListener('keydown', handleKeyDown);
    document.addEventListener('contextmenu', handleContextMenu);

    return () => {
      document.removeEventListener('keydown', handleKeyDown);
      document.removeEventListener('contextmenu', handleContextMenu);
    };
  }, []);
};