import { useState, useCallback, useRef, useEffect } from 'react';
import styles from './AppLayout.module.css';

interface Props {
  left: React.ReactNode;
  center: React.ReactNode;
  right: React.ReactNode;
}

const MIN_PANEL = 200;
const DEFAULT_LEFT = 280;
const DEFAULT_RIGHT = 320;

export function AppLayout({ left, center, right }: Props) {
  const [leftWidth, setLeftWidth] = useState(DEFAULT_LEFT);
  const [rightWidth, setRightWidth] = useState(DEFAULT_RIGHT);
  const containerRef = useRef<HTMLDivElement>(null);
  const dragging = useRef<'left' | 'right' | null>(null);

  const onMouseDown = useCallback((side: 'left' | 'right') => {
    dragging.current = side;
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
  }, []);

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (!dragging.current || !containerRef.current) return;
      const rect = containerRef.current.getBoundingClientRect();

      if (dragging.current === 'left') {
        const w = Math.max(MIN_PANEL, Math.min(e.clientX - rect.left, rect.width - rightWidth - MIN_PANEL));
        setLeftWidth(w);
      } else {
        const w = Math.max(MIN_PANEL, Math.min(rect.right - e.clientX, rect.width - leftWidth - MIN_PANEL));
        setRightWidth(w);
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
      <div className={styles.panel} style={{ width: leftWidth }}>
        {left}
      </div>
      <div
        className={styles.divider}
        onMouseDown={() => onMouseDown('left')}
      />
      <div className={styles.center}>
        {center}
      </div>
      <div
        className={styles.divider}
        onMouseDown={() => onMouseDown('right')}
      />
      <div className={styles.panel} style={{ width: rightWidth }}>
        {right}
      </div>
    </div>
  );
}
