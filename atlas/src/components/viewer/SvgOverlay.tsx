import { useAtlasStore } from '../../store/atlasStore';
import { useSelectRegion } from '../../hooks/useSelectRegion';
import type { SvgRegion } from '../../types';
import styles from './SvgOverlay.module.css';

interface Props {
  regions: SvgRegion[];
  viewBox: { width: number; height: number };
  displayWidth: number;
  displayHeight: number;
}

export function SvgOverlay({ regions, viewBox, displayWidth, displayHeight }: Props) {
  const { selectedStructureId, hoveredStructureId, setHovered } = useAtlasStore();
  const selectRegion = useSelectRegion();

  const viewBoxStr = `0 0 ${viewBox.width} ${viewBox.height}`;

  return (
    <svg
      className={styles.overlay}
      viewBox={viewBoxStr}
      width={displayWidth}
      height={displayHeight}
      preserveAspectRatio="none"
    >
      {regions.map((region, i) => {
        const isSelected = region.structureId === selectedStructureId;
        const isHovered = region.structureId === hoveredStructureId && !isSelected;

        return (
          <path
            key={i}
            d={region.d}
            fill={isSelected ? 'rgba(124, 58, 237, 0.40)' : 'transparent'}
            stroke={isSelected ? '#7c3aed' : isHovered ? region.strokeColor : 'transparent'}
            strokeWidth={isSelected ? 2.5 : isHovered ? 5 : 0}
            className={styles.regionPath}
            onMouseEnter={() => setHovered(region.structureId)}
            onMouseLeave={() => setHovered(null)}
            onClick={(e) => {
              e.stopPropagation();
              selectRegion(region.structureId);
            }}
          />
        );
      })}
    </svg>
  );
}
