import { useRef, useEffect, useState, useCallback } from 'react';
import { useAtlasStore } from '../../store/atlasStore';
import { getSectionThumbnailUrl } from '../../api/allen';
import styles from './SectionStrip.module.css';

const THUMB_WIDTH = 72; // thumb width + gap (64 img + 4 padding + 4 gap)

export function SectionStrip() {
  const { sections, currentSectionIndex, setCurrentSection } = useAtlasStore();
  const stripRef = useRef<HTMLDivElement>(null);
  const [visibleRange, setVisibleRange] = useState({ start: 0, end: 40 });

  // Calculate visible range based on scroll position
  const updateVisibleRange = useCallback(() => {
    const el = stripRef.current;
    if (!el) return;
    const scrollLeft = el.scrollLeft;
    const viewportWidth = el.clientWidth;
    const buffer = 10; // render 10 extra on each side
    const start = Math.max(0, Math.floor(scrollLeft / THUMB_WIDTH) - buffer);
    const end = Math.min(sections.length, Math.ceil((scrollLeft + viewportWidth) / THUMB_WIDTH) + buffer);
    setVisibleRange({ start, end });
  }, [sections.length]);

  // Scroll to active thumbnail when section changes
  useEffect(() => {
    const el = stripRef.current;
    if (!el) return;
    const targetScroll = currentSectionIndex * THUMB_WIDTH - el.clientWidth / 2 + THUMB_WIDTH / 2;
    el.scrollTo({ left: Math.max(0, targetScroll), behavior: 'smooth' });
  }, [currentSectionIndex]);

  // Update visible range on scroll and on mount
  useEffect(() => {
    const el = stripRef.current;
    if (!el) return;
    updateVisibleRange();
    const onScroll = () => updateVisibleRange();
    el.addEventListener('scroll', onScroll, { passive: true });
    return () => el.removeEventListener('scroll', onScroll);
  }, [updateVisibleRange]);

  // Recalculate when sections change (view plane switch)
  useEffect(() => {
    updateVisibleRange();
  }, [sections, updateVisibleRange]);

  const totalWidth = sections.length * THUMB_WIDTH;

  return (
    <div className={styles.strip} ref={stripRef}>
      {/* Spacer for items before visible range */}
      <div style={{ minWidth: visibleRange.start * THUMB_WIDTH, flexShrink: 0 }} />
      {sections.slice(visibleRange.start, visibleRange.end).map((sec, i) => {
        const idx = visibleRange.start + i;
        return (
          <button
            key={sec.id}
            className={`${styles.thumb} ${idx === currentSectionIndex ? styles.active : ''}`}
            onClick={() => setCurrentSection(idx)}
            title={`Image ${idx + 1} of ${sections.length} (Allen section ${sec.section_number})`}
          >
            <img
              src={getSectionThumbnailUrl(sec.id, sec)}
              alt={`Image ${idx + 1}`}
              width={64}
              height={48}
              loading="lazy"
            />
            <span className={styles.number}>{idx + 1}</span>
          </button>
        );
      })}
      {/* Spacer for items after visible range */}
      <div style={{ minWidth: Math.max(0, (sections.length - visibleRange.end) * THUMB_WIDTH), flexShrink: 0 }} />
    </div>
  );
}
