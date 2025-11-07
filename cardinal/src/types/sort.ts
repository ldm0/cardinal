export type SortKey = 'fullPath' | 'size' | 'mtime' | 'ctime';

export type SortDirection = 'asc' | 'desc';

export type SortState =
  | {
      key: SortKey;
      direction: SortDirection;
    }
  | null;
