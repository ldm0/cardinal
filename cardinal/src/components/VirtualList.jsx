import React, { useRef, useState, useCallback, useLayoutEffect, useEffect, forwardRef, useImperativeHandle, useMemo } from 'react';
import { SCROLLBAR_THUMB_MIN } from '../constants';

/**
 * 虚拟滚动列表组件
 * 
 * 特性:
 * - 虚拟滚动，只渲染可见区域
 * - 支持大数据量
 * - 自定义滚动条
 * - 水平滚动同步
 */
export const VirtualList = forwardRef(function VirtualList({
	rowCount = 0,
	rowHeight = 24,
	overscan = 5,
	renderRow,
	onRangeChange,
	onScrollSync,
	className = ''
}, ref) {
	const containerRef = useRef(null);
	const viewportRef = useRef(null);
	const scrollTrackRef = useRef(null);
	const scrollThumbRef = useRef(null);
	const isDraggingRef = useRef(false);
	const hideTimerRef = useRef(null);

	// 自动隐藏滚动条的延迟（ms）
	const HIDE_DELAY = 900;
	const lastScrollLeftRef = useRef(0);
	
	const [scrollTop, setScrollTop] = useState(0);
	const [viewportHeight, setViewportHeight] = useState(0);
	const [range, setRange] = useState({ start: 0, end: -1 });

	// 计算总虚拟高度和滚动范围
	const { totalHeight, maxScrollTop } = useMemo(() => ({
		totalHeight: rowCount * rowHeight,
		maxScrollTop: Math.max(0, rowCount * rowHeight - viewportHeight)
	}), [rowCount, rowHeight, viewportHeight]);
	
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
		setRange(prev => {
			const changed = prev.start !== nextRange.start || prev.end !== nextRange.end;
			if (changed && onRangeChange && nextRange.end >= nextRange.start && rowCount > 0) {
				onRangeChange(nextRange.start, nextRange.end);
			}
			return changed ? nextRange : prev;
		});
	}, [onRangeChange, rowCount]);

	// 滚动条显示/隐藏辅助函数
	const showScrollbar = useCallback(() => {
		const el = containerRef.current;
		if (!el) return;
		el.classList.add('show-scrollbar');
		if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
	}, []);

	const scheduleHideScrollbar = useCallback(() => {
		if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
		hideTimerRef.current = setTimeout(() => {
			if (!isDraggingRef.current) {
				containerRef.current?.classList.remove('show-scrollbar');
			}
		}, HIDE_DELAY);
	}, []);

	// 更新滚动条外观
	const updateScrollbar = useCallback((scrollTop) => {
		const track = scrollTrackRef.current;
		const thumb = scrollThumbRef.current;
		
		if (!track || !thumb) return;
		
		const trackHeight = track.clientHeight;
		const shouldShow = totalHeight > viewportHeight && trackHeight > 0;
		
		if (!shouldShow) {
			thumb.style.display = 'none';
			return;
		}
		
		thumb.style.display = 'block';
		const thumbHeight = Math.max(SCROLLBAR_THUMB_MIN, (viewportHeight / totalHeight) * trackHeight);
		const scrollRatio = scrollTop / maxScrollTop;
		const thumbTop = scrollRatio * (trackHeight - thumbHeight);
		
		thumb.style.height = `${thumbHeight}px`;
		thumb.style.transform = `translateY(${thumbTop}px)`;
	}, [totalHeight, viewportHeight, maxScrollTop]);

	// 更新滚动位置和范围 - 合并逻辑
	const updateScrollAndRange = useCallback((nextScrollTop) => {
		const clamped = Math.max(0, Math.min(nextScrollTop, maxScrollTop));
		setScrollTop(clamped);
		setRangeIfChanged(computeRange(clamped, viewportHeight));
		updateScrollbar(clamped);
	}, [maxScrollTop, computeRange, viewportHeight, updateScrollbar, setRangeIfChanged]);

	// 事件处理函数
	const handleWheel = useCallback((e) => {
		e.preventDefault();
		showScrollbar();
		scheduleHideScrollbar();
		updateScrollAndRange(scrollTop + e.deltaY);
	}, [scrollTop, updateScrollAndRange, showScrollbar, scheduleHideScrollbar]);

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
		showScrollbar();
		
		const trackRect = track.getBoundingClientRect();
		const thumbRect = thumb.getBoundingClientRect();
		const trackHeight = trackRect.height;
		const thumbHeight = thumbRect.height;
		
		// 计算鼠标在thumb内的相对位置
		const mouseOffsetInThumb = e.clientY - thumbRect.top;
		
		const handleMouseMove = (moveEvent) => {
			if (!isDraggingRef.current) return;
			
			// 计算鼠标相对于track顶部的位置，减去在thumb内的偏移
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
			scheduleHideScrollbar();
			document.removeEventListener('mousemove', handleMouseMove);
			document.removeEventListener('mouseup', handleMouseUp);
		};
		
		document.addEventListener('mousemove', handleMouseMove);
		document.addEventListener('mouseup', handleMouseUp);
	}, [maxScrollTop, updateScrollAndRange, showScrollbar, scheduleHideScrollbar]);

	const handleTrackClick = useCallback((e) => {
		if (e.target === scrollThumbRef.current) return;
		
		const rect = scrollTrackRef.current?.getBoundingClientRect();
		if (!rect) return;
		
		const clickY = e.clientY - rect.top;
		const scrollRatio = clickY / rect.height;
		const newScrollTop = scrollRatio * maxScrollTop;
		updateScrollAndRange(newScrollTop);
	}, [maxScrollTop, updateScrollAndRange]);

	// 监听容器尺寸变化
	useLayoutEffect(() => {
		const container = containerRef.current;
		if (!container) return;
		
		const updateViewport = () => {
			const newHeight = container.clientHeight;
			setViewportHeight(newHeight);
		};
		
		const resizeObserver = new ResizeObserver(updateViewport);
		resizeObserver.observe(container);
		updateViewport(); // 初始更新
		
		return () => resizeObserver.disconnect();
	}, []);

	// 当参数变化时重新计算
	useEffect(() => {
		if (viewportHeight > 0) {
			setRangeIfChanged(computeRange(scrollTop, viewportHeight));
			updateScrollbar(scrollTop);
		}
	}, [rowCount, rowHeight, overscan, viewportHeight, scrollTop, computeRange, updateScrollbar, setRangeIfChanged]);

	// 暴露的API
	useImperativeHandle(ref, () => ({
		scrollToTop: () => updateScrollAndRange(0),
		scrollToIndex: (index, align = 'start') => {
			if (index < 0 || index >= rowCount) return;
			const targetTop = index * rowHeight;
			let next = targetTop;
			if (align === 'center') next = targetTop - (viewportHeight - rowHeight) / 2;
			else if (align === 'end') next = targetTop - (viewportHeight - rowHeight);
			updateScrollAndRange(next);
		},
		getScrollTop: () => scrollTop
	}), [rowCount, rowHeight, viewportHeight, scrollTop, updateScrollAndRange]);

	// 渲染的项目
	const renderedItems = useMemo(() => {
		const { start, end } = range;
		if (!(rowCount > 0 && end >= start)) return null;
		const count = end - start + 1;
		const offsetTop = start * rowHeight - scrollTop;
		return Array.from({ length: count }, (_, i) => {
			const rowIndex = start + i;
			return renderRow(rowIndex, {
				position: 'absolute',
				top: offsetTop + i * rowHeight,
				height: rowHeight,
				left: 0,
				right: 0
			});
		});
	}, [range, rowCount, rowHeight, scrollTop, renderRow]);

	return (
		<div 
			ref={containerRef}
			className={`virtual-list ${className}`}
			onWheel={handleWheel}
			role="list"
			aria-rowcount={rowCount}
		>
			{/* 水平滚动视口 */}
			<div 
				ref={viewportRef}
				className="virtual-list-viewport"
				onScroll={handleHorizontalScroll}
			>
				<div className="virtual-list-items">
					{renderedItems}
				</div>
			</div>
			
			{/* 虚拟滚动条 */}
			<div 
				className="virtual-scrollbar"
				onMouseEnter={showScrollbar}
				onMouseLeave={() => { if (!isDraggingRef.current) scheduleHideScrollbar(); }}
			>
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