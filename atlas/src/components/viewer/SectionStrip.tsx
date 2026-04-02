import { useRef, useEffect } from 'react';
import { useAtlasStore } from '../../store/atlasStore';
import { getSectionImageUrl } from '../../api/allen';
import styles from './SectionStrip.module.css';

export function SectionStrip() {
  const { sections, currentSectionIndex, setCurrentSection } = useAtlasStore();
  const stripRef = useRef<HTMLDivElement>(null);
  const activeRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (activeRef.current && stripRef.current) {
      activeRef.current.scrollIntoView({
        behavior: 'smooth',
        block: 'nearest',
        inline: 'center',
      });
    }
  }, [currentSectionIndex]);

  return (
    <div className={styles.strip} ref={stripRef}>
      {sections.map((sec, i) => (
        <button
          key={sec.id}
          ref={i === currentSectionIndex ? activeRef : undefined}
          className={`${styles.thumb} ${i === currentSectionIndex ? styles.active : ''}`}
          onClick={() => setCurrentSection(i)}
          title={`Image ${i + 1} of ${sections.length} (Allen section ${sec.section_number})`}
        >
          <img
            src={getSectionImageUrl(sec.id, 6)}
            alt={`Image ${i + 1}`}
            width={64}
            height={48}
            loading="lazy"
          />
          <span className={styles.number}>{i + 1}</span>
        </button>
      ))}
    </div>
  );
}
