import { useAtlasStore } from '../../store/atlasStore';
import { findNode } from '../../utils/treeUtils';
import styles from './RegionTooltip.module.css';

interface Props {
  structureId: number;
  x: number;
  y: number;
}

export function RegionTooltip({ structureId, x, y }: Props) {
  const ontology = useAtlasStore((s) => s.ontology);
  const node = ontology ? findNode(ontology, structureId) : null;

  if (!node) return null;

  return (
    <div
      className={styles.tooltip}
      style={{
        left: x + 16,
        top: y - 12,
      }}
    >
      <div className={styles.name}>{node.n}</div>
      <div className={styles.acronym}>{node.a}</div>
    </div>
  );
}
