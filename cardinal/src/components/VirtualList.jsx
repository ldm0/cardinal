// React & libs
import React, {
	useRef,
	useState,
	useCallback,
	useLayoutEffect,
	useEffect,
	forwardRef,
	useImperativeHandle,
	useMemo
} from 'react';
import { invoke } from '@tauri-apps/api/core';
// Internal constants
import { SCROLLBAR_THUMB_MIN } from '../constants';

/**
 * 虚拟滚动列表组件（含行数据按需加载缓存）
 */
export const VirtualList = forwardRef(function VirtualList({
	results = null,
	rowHeight = 24,
	overscan = 5,
	renderRow,
	onScrollSync,
	className = ''
}, ref) {
	// ----- refs -----
	const containerRef = useRef(null);
	const viewportRef = useRef(null);
	const scrollTrackRef = useRef(null);
	const scrollThumbRef = useRef(null);
	const isDraggingRef = useRef(false);
	const lastScrollLeftRef = useRef(0);
	const loadingRef = useRef(new Set());

	// ----- state -----
	const [cache, setCache] = useState(() => new Map());
	const [scrollTop, setScrollTop] = useState(0);
	const [viewportHeight, setViewportHeight] = useState(0);
	const [range, setRange] = useState({ start: 0, end: -1 });

	// ----- derived -----
	// 行数直接来自 results（不再支持显式 rowCount）
	const rowCount = results?.length ?? 0;
	// 计算总虚拟高度和滚动范围
	const { totalHeight, maxScrollTop } = useMemo(() => ({
		totalHeight: rowCount * rowHeight,
		maxScrollTop: Math.max(0, rowCount * rowHeight - viewportHeight)
	}), [rowCount, rowHeight, viewportHeight]);

	// ----- callbacks: pure calculations first -----
	// 计算可见范围
	const computeRange = useCallback((currentScrollTop, vh) => {
		if (!rowCount || !vh) return { start: 0, end: -1 };
		const startIndex = Math.floor(currentScrollTop / rowHeight);
		const endIndex = startIndex + Math.ceil(vh / rowHeight) - 1;
		return {
			start: Math.max(0, startIndex - overscan),
			end: Math.min(rowCount - 1, endIndex + overscan)
		};
	}, [rowCount, rowHeight, overscan]);

	// 统一的 range 更新封装
	const setRangeIfChanged = useCallback((nextRange) => {
		setRange(prev => (prev.start !== nextRange.start || prev.end !== nextRange.end) ? nextRange : prev);
	}, []);

	// 滚动条更新
	const updateScrollbar = useCallback((st) => {
		const track = scrollTrackRef.current;
		const thumb = scrollThumbRef.current;
		if (!track || !thumb || totalHeight <= viewportHeight) {
			if (thumb) thumb.style.display = 'none';
			return;
		}
		thumb.style.display = 'block';
		const trackHeight = track.clientHeight;
		const thumbHeight = Math.max(SCROLLBAR_THUMB_MIN, (viewportHeight / totalHeight) * trackHeight);
		const thumbTop = maxScrollTop > 0 ? (st / maxScrollTop) * (trackHeight - thumbHeight) : 0;
		thumb.style.height = `${thumbHeight}px`;
		thumb.style.transform = `translateY(${thumbTop}px)`;
	}, [totalHeight, viewportHeight, maxScrollTop]);

	// 更新滚动位置和范围
	const updateScrollAndRange = useCallback((nextScrollTop) => {
		const clamped = Math.max(0, Math.min(nextScrollTop, maxScrollTop));
		setScrollTop(clamped);
		setRangeIfChanged(computeRange(clamped, viewportHeight));
		updateScrollbar(clamped);
	}, [maxScrollTop, computeRange, viewportHeight, updateScrollbar, setRangeIfChanged]);

	// ----- data loading -----
	// 内置行数据加载
	const ensureRangeLoaded = useCallback(async (start, end) => {
		if (!results || start < 0 || end < start || rowCount === 0) return;
		const needLoading = [];
		for (let i = start; i <= end; i++) {
			if (!cache.has(i) && !loadingRef.current.has(i)) {
				needLoading.push(i);
				loadingRef.current.add(i);
			}
		}
		if (needLoading.length === 0) return;
		try {
			const slice = needLoading.map(i => results[i]);
			const fetched = await invoke('get_nodes_info', { results: slice });
			setCache(prev => {
				const newCache = new Map(prev);
				needLoading.forEach((originalIndex, idx) => {
					newCache.set(originalIndex, fetched[idx]);
					loadingRef.current.delete(originalIndex);
				});
				return newCache;
			});
		} catch (err) {
			needLoading.forEach(i => loadingRef.current.delete(i));
			console.error('Failed loading rows', err);
		}
	}, [results, rowCount, cache]);

	// ----- event handlers -----
	// 垂直滚动（阻止默认以获得一致行为）
	const handleWheel = useCallback((e) => {
		e.preventDefault();
		updateScrollAndRange(scrollTop + e.deltaY);
	}, [scrollTop, updateScrollAndRange]);

	// 水平滚动同步
	const handleHorizontalScroll = useCallback((e) => {
		const scrollLeft = e.target.scrollLeft;
		if (onScrollSync && scrollLeft !== lastScrollLeftRef.current) {
			lastScrollLeftRef.current = scrollLeft;
			onScrollSync(scrollLeft);
		}
	}, [onScrollSync]);

	// 滚动条拖拽处理
	const handleThumbMouseDown = useCallback((e) => {
		e.preventDefault();
		isDraggingRef.current = true;
		const track = scrollTrackRef.current;
		const thumb = scrollThumbRef.current;
		if (!track || !thumb) return;
		// 添加拖拽状态样式（使 track 始终保持 hover 高亮）
		track.classList.add('is-dragging');
		const trackRect = track.getBoundingClientRect();
		const thumbRect = thumb.getBoundingClientRect();
		const trackHeight = trackRect.height;
		const thumbHeight = thumbRect.height;
		// 计算鼠标在 thumb 内的相对位置
		const mouseOffsetInThumb = e.clientY - thumbRect.top;
		const handleMouseMove = (moveEvent) => {
			if (!isDraggingRef.current) return;
			// 计算鼠标相对于 track 顶部的位置，减去在 thumb 内的偏移
			const mousePositionInTrack = moveEvent.clientY - trackRect.top - mouseOffsetInThumb;
			// 计算滚动比例，限制在有效范围内
			const maxThumbTop = trackHeight - thumbHeight;
			const clampedThumbTop = Math.max(0, Math.min(mousePositionInTrack, maxThumbTop));
			const scrollRatio = maxThumbTop > 0 ? clampedThumbTop / maxThumbTop : 0;
			const newScrollTop = scrollRatio * maxScrollTop;
			updateScrollAndRange(newScrollTop);
		};
		const handleMouseUp = () => {
			isDraggingRef.current = false;
			// 移除拖拽状态样式
			track.classList.remove('is-dragging');
			document.removeEventListener('mousemove', handleMouseMove);
			document.removeEventListener('mouseup', handleMouseUp);
		};
		document.addEventListener('mousemove', handleMouseMove);
		document.addEventListener('mouseup', handleMouseUp);
	}, [maxScrollTop, updateScrollAndRange]);

	// 滚动条轨道点击（跳转）
	const handleTrackClick = useCallback((e) => {
		if (e.target === scrollThumbRef.current) return;
		const rect = scrollTrackRef.current?.getBoundingClientRect();
		if (!rect) return;
		const clickY = e.clientY - rect.top;
		const scrollRatio = clickY / rect.height;
		const newScrollTop = scrollRatio * maxScrollTop;
		updateScrollAndRange(newScrollTop);
	}, [maxScrollTop, updateScrollAndRange]);

	// ----- effects -----
	// 结果集变化时重置缓存
	useEffect(() => { // results change -> reset cache
		setCache(new Map());
		loadingRef.current.clear();
	}, [results]);

	// range 变化时自动加载
	useEffect(() => { // auto load
		if (range.end >= range.start) ensureRangeLoaded(range.start, range.end);
	}, [range, ensureRangeLoaded]);

	// 监听容器尺寸变化
	useLayoutEffect(() => { // observe container height
		const container = containerRef.current;
		if (!container) return;
		const updateViewport = () => setViewportHeight(container.clientHeight);
		const resizeObserver = new ResizeObserver(updateViewport);
		resizeObserver.observe(container);
		updateViewport();
		return () => resizeObserver.disconnect();
	}, []);

	// 当参数变化时重新计算
	useEffect(() => { // recompute on deps
		if (viewportHeight > 0) {
			setRangeIfChanged(computeRange(scrollTop, viewportHeight));
			updateScrollbar(scrollTop);
		}
	}, [rowCount, rowHeight, overscan, viewportHeight, scrollTop, computeRange, updateScrollbar, setRangeIfChanged]);

	// ----- imperative API -----
	// 暴露的API
	useImperativeHandle(ref, () => ({
		scrollToTop: () => updateScrollAndRange(0),
		ensureRangeLoaded,
	}), [updateScrollAndRange, ensureRangeLoaded]);

	// ----- rendered items memo -----
	// 渲染的项目
	const renderedItems = useMemo(() => {
		const { start, end } = range;
		if (!(rowCount > 0 && end >= start)) return null;
		const count = end - start + 1;
		const offsetTop = start * rowHeight - scrollTop;
		return Array.from({ length: count }, (_, i) => {
			const rowIndex = start + i;
			const item = cache.get(rowIndex);
			return renderRow(rowIndex, item, {
				position: 'absolute',
				top: offsetTop + i * rowHeight,
				height: rowHeight,
				left: 0,
				right: 0
			});
		});
	}, [range, rowCount, rowHeight, scrollTop, renderRow, cache]);

	// ----- render -----
	return (
		<div
			ref={containerRef}
			className={`virtual-list ${className}`}
			onWheel={handleWheel}
			role="list"
			aria-rowcount={rowCount}
		>
			<div
				ref={viewportRef}
				className="virtual-list-viewport"
				onScroll={handleHorizontalScroll}
			>
				<div className="virtual-list-items">
					{renderedItems}
				</div>
			</div>
			<div className="virtual-scrollbar">
				<div
					ref={scrollTrackRef}
					className="virtual-scrollbar-track"
					onClick={handleTrackClick}
				>
					<div
						ref={scrollThumbRef}
						className="virtual-scrollbar-thumb"
						onMouseDown={handleThumbMouseDown}
					/>
				</div>
			</div>
		</div>
	);
});

VirtualList.displayName = 'VirtualList';

export default VirtualList;