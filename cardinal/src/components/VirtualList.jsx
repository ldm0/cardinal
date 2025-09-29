// React & libs
import React, {
	useRef,
	useState,
	useCallback,
	useLayoutEffect,
	useEffect,
	forwardRef,
	useImperativeHandle
} from 'react';
import Scrollbar from './Scrollbar';
import { useDataLoader } from '../hooks/useDataLoader';

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
	const lastScrollLeftRef = useRef(0);

	// ----- state -----
	const [scrollTop, setScrollTop] = useState(0);
	const [viewportHeight, setViewportHeight] = useState(0);

	// ----- derived -----
	// 行数直接来自 results（不再支持显式 rowCount）
	const rowCount = results?.length ?? 0;

	// ----- data loader -----
	const { cache, ensureRangeLoaded } = useDataLoader(results);

	// 计算总虚拟高度和滚动范围
	const totalHeight = rowCount * rowHeight;
	const maxScrollTop = Math.max(0, totalHeight - viewportHeight);

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

	// 更新滚动位置和范围
	const updateScrollAndRange = useCallback((nextScrollTop) => {
		const clamped = Math.max(0, Math.min(nextScrollTop, maxScrollTop));
		setScrollTop(prev => (prev === clamped ? prev : clamped));
	}, [maxScrollTop]);

	const { start, end } = computeRange(scrollTop, viewportHeight);

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

	// ----- effects -----
	// 可见窗口变化时自动加载
	useEffect(() => { // auto load
		if (end >= start) ensureRangeLoaded(start, end);
	}, [start, end, ensureRangeLoaded]);

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

	// 当滚动范围变化时校正位置
	useEffect(() => {
		setScrollTop(prev => {
			const clamped = Math.max(0, Math.min(prev, maxScrollTop));
			return clamped === prev ? prev : clamped;
		});
	}, [maxScrollTop]);

	// ----- imperative API -----
	// 暴露的API
	useImperativeHandle(ref, () => ({
		scrollToTop: () => updateScrollAndRange(0),
		ensureRangeLoaded,
	}), [updateScrollAndRange, ensureRangeLoaded]);

	// ----- rendered items memo -----
	// 渲染的项目
	let renderedItems = null;
	if (rowCount > 0 && end >= start) {
		const count = end - start + 1;
		const offsetTop = start * rowHeight - scrollTop;
		renderedItems = Array.from({ length: count }, (_, i) => {
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
	}

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
				className="virtual-list-viewport"
				onScroll={handleHorizontalScroll}
			>
				<div className="virtual-list-items">
					{renderedItems}
				</div>
			</div>
			<Scrollbar
				totalHeight={totalHeight}
				viewportHeight={viewportHeight}
				maxScrollTop={maxScrollTop}
				scrollTop={scrollTop}
				onScrollUpdate={updateScrollAndRange}
			/>
		</div>
	);
});

VirtualList.displayName = 'VirtualList';

export default VirtualList;