import { useCallback, useRef, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

/**
 * 数据加载 hook，用于按需加载虚拟列表的数据
 */
export function useDataLoader(results) {
    const loadingRef = useRef(new Set());
    const versionRef = useRef(0);
    const cacheRef = useRef();
    const [cache, setCache] = useState(() => {
        const initial = new Map();
        cacheRef.current = initial;
        return initial;
    });
    const resultsRef = useRef([]);

    // 当 results 变化时清除加载状态
    useEffect(() => {
        versionRef.current += 1;
        loadingRef.current.clear();
        const nextCache = new Map();
        cacheRef.current = nextCache;
        resultsRef.current = Array.isArray(results) ? results : [];
        setCache(nextCache);
    }, [results]);

    useEffect(() => {
        let unlistenIconUpdate;
        (async () => {
            try {
                unlistenIconUpdate = await listen('icon_update', (event) => {
                    const updates = event?.payload;
                    setCache(prev => {
                        const next = new Map(prev);
                        updates.forEach(update => {
                            const index = resultsRef.current.indexOf(update.slabIndex);
                            if (index === -1) return;
                            const current = next.get(index);
                            next.set(index, current ? { ...current, icon: update.icon } : { icon: update.icon });
                        });
                        cacheRef.current = next;
                        return next;
                    });
                });
            } catch (error) {
                console.error('Failed to listen icon_update', error);
            }
        })();
        return () => {
            unlistenIconUpdate?.();
        };
    }, []);

    const ensureRangeLoaded = useCallback(async (start, end) => {
        const list = resultsRef.current;
        const total = list.length;
        if (start < 0 || end < start || total === 0) return;
        const needLoading = [];
        for (let i = start; i <= end && i < total; i++) {
            if (!cacheRef.current.has(i) && !loadingRef.current.has(i) && list[i] != null) {
                needLoading.push(i);
                loadingRef.current.add(i);
            }
        }
        if (needLoading.length === 0) return;
        const versionAtRequest = versionRef.current;
        try {
            const slice = needLoading.map(i => list[i]);
            const fetched = await invoke('get_nodes_info', { results: slice });
            if (versionRef.current !== versionAtRequest) {
                needLoading.forEach(i => loadingRef.current.delete(i));
                return;
            }
            setCache(prev => {
                if (versionRef.current !== versionAtRequest) return prev;
                const newCache = new Map(prev);
                needLoading.forEach((originalIndex, idx) => {
                    const item = fetched[idx];
                    if (item) {
                        const existing = newCache.get(originalIndex);
                        const icon = existing?.icon ?? item.icon;
                        newCache.set(originalIndex, icon ? { ...item, icon } : item);
                    }
                    loadingRef.current.delete(originalIndex);
                });
                cacheRef.current = newCache;
                return newCache;
            });
        } catch (err) {
            needLoading.forEach(i => loadingRef.current.delete(i));
            console.error('Failed loading rows', err);
        }
    }, [results]);

    return { cache, ensureRangeLoaded };
}
