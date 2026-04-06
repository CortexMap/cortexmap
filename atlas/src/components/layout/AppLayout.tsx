import { useState, useCallback, useRef, useEffect } from 'react';
import styles from './AppLayout.module.css';

interface Props {
  left: React.ReactNode;
  center: React.ReactNode;
  right: React.ReactNode;
  bottom?: React.ReactNode;
}

const MIN_PANEL = 200;
const DEFAULT_LEFT = 280;
const DEFAULT_RIGHT = 320;
const MIN_BOTTOM = 48;
const DEFAULT_BOTTOM = 64;

export function AppLayout({ left, center, right, bottom }: Props) {
  const [leftWidth, setLeftWidth] = useState(DEFAULT_LEFT);
  const [rightWidth, setRightWidth] = useState(DEFAULT_RIGHT);
  const [bottomHeight, setBottomHeight] = useState(DEFAULT_BOTTOM);
  const [leftCollapsed, setLeftCollapsed] = useState(false);
  const [rightCollapsed, setRightCollapsed] = useState(false);
  const [bottomCollapsed, setBottomCollapsed] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const dragging = useRef<'left' | 'right' | 'bottom' | null>(null);

  const onMouseDown = useCallback((side: 'left' | 'right' | 'bottom') => {
    dragging.current = side;
    document.body.style.cursor = side === 'bottom' ? 'row-resize' : 'col-resize';
    document.body.style.userSelect = 'none';
  }, []);

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (!dragging.current || !containerRef.current) return;
      const rect = containerRef.current.getBoundingClientRect();

      if (dragging.current === 'left') {
        const w = Math.max(MIN_PANEL, Math.min(e.clientX - rect.left, rect.width - rightWidth - MIN_PANEL));
        setLeftWidth(w);
      } else if (dragging.current === 'right') {
        const w = Math.max(MIN_PANEL, Math.min(rect.right - e.clientX, rect.width - leftWidth - MIN_PANEL));
        setRightWidth(w);
      } else if (dragging.current === 'bottom') {
        const h = Math.max(MIN_BOTTOM, Math.min(rect.bottom - e.clientY, rect.height * 0.5));
        setBottomHeight(h);
      }
    };

    const onUp = () => {
      dragging.current = null;
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };

    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
    return () => {
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
    };
  }, [leftWidth, rightWidth]);

  return (
    <div className={styles.layout} ref={containerRef}>
      {/* Top row: left | center | right */}
      <div className={styles.topRow}>
        {!leftCollapsed && (
          <div className={styles.panel} style={{ width: leftWidth }}>
            {left}
          </div>
        )}
        <div
          className={`${styles.dividerRail} ${leftCollapsed ? styles.collapsed : ''}`}
          onMouseDown={leftCollapsed ? undefined : () => onMouseDown('left')}
        >
          <button
            className={styles.collapseBtn}
            onClick={() => setLeftCollapsed(!leftCollapsed)}
            title={leftCollapsed ? 'Show region tree' : 'Hide region tree'}
          >
            {leftCollapsed ? '\u25B6' : '\u25C0'}
          </button>
        </div>
        <div className={styles.center}>
          {center}
        </div>
        <div
          className={`${styles.dividerRail} ${rightCollapsed ? styles.collapsed : ''}`}
          onMouseDown={rightCollapsed ? undefined : () => onMouseDown('right')}
        >
          <button
            className={styles.collapseBtn}
            onClick={() => setRightCollapsed(!rightCollapsed)}
            title={rightCollapsed ? 'Show region details' : 'Hide region details'}
          >
            {rightCollapsed ? '\u25C0' : '\u25B6'}
          </button>
        </div>
        {!rightCollapsed && (
          <div className={styles.panel} style={{ width: rightWidth }}>
            {right}
          </div>
        )}
      </div>

      {/* Bottom panel: dashboard/controls */}
      {bottom && (
        <>
          <div
            className={`${styles.dividerRailH} ${bottomCollapsed ? styles.collapsed : ''}`}
            onMouseDown={bottomCollapsed ? undefined : () => onMouseDown('bottom')}
          >
            <button
              className={styles.collapseBtnH}
              onClick={() => setBottomCollapsed(!bottomCollapsed)}
              title={bottomCollapsed ? 'Show dashboard' : 'Hide dashboard'}
            >
              {bottomCollapsed ? '\u25B2' : '\u25BC'}
            </button>
          </div>
          {!bottomCollapsed && (
            <div className={styles.bottomPanel} style={{ height: bottomHeight }}>
              {bottom}
            </div>
          )}
        </>
      )}
    </div>
  );
}
