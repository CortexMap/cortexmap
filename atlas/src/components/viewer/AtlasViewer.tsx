import { useEffect, useRef, useCallback, useState } from 'react';
import { useAtlasStore } from '../../store/atlasStore';
import type { ViewPlane } from '../../store/atlasStore';
import { getSectionImageUrl, fetchSvgAnnotation } from '../../api/allen';
import { SvgOverlay } from './SvgOverlay';
import { RegionTooltip } from './RegionTooltip';
import { SectionStrip } from './SectionStrip';
import { parseSvgRegions } from '../../utils/svgParser';
import { getDownsampleForZoom } from '../../utils/imageUtils';
import type { SvgRegion } from '../../types';
import styles from './AtlasViewer.module.css';

export function AtlasViewer() {
  const {
    sections, currentSectionIndex, zoomLevel, panX, panY, viewPlane,
    setZoom, setPan, resetView, setImageLoaded, setAnnotatedStructures,
    cacheSectionAnnotations, hoveredStructureId, navigating, setViewPlane,
  } = useAtlasStore();

  const section = sections[currentSectionIndex];
  const containerRef = useRef<HTMLDivElement>(null);
  const isPanning = useRef(false);
  const lastMouse = useRef({ x: 0, y: 0 });

  const [svgRegions, setSvgRegions] = useState<SvgRegion[]>([]);
  const [svgViewBox, setSvgViewBox] = useState({ width: 0, height: 0 });
  const [svgLoading, setSvgLoading] = useState(false);
  const [tooltipPos, setTooltipPos] = useState<{ x: number; y: number } | null>(null);

  const downsample = getDownsampleForZoom(zoomLevel);
  const imageUrl = section ? getSectionImageUrl(section.id, downsample, section) : '';

  // Fetch SVG annotations when section changes
  useEffect(() => {
    if (!section) return;
    let cancelled = false;
    setSvgLoading(true);

    fetchSvgAnnotation(section.id).then((svgStr) => {
      if (cancelled) return;
      const parsed = parseSvgRegions(svgStr);
      setSvgRegions(parsed.regions);
      setSvgViewBox(parsed.viewBox);
      setAnnotatedStructures(parsed.structureIds);
      cacheSectionAnnotations(currentSectionIndex, parsed.structureIds);
      setSvgLoading(false);
    }).catch(() => {
      if (!cancelled) setSvgLoading(false);
    });

    return () => { cancelled = true; };
  }, [section?.id, setAnnotatedStructures, cacheSectionAnnotations, currentSectionIndex]);

  // Pan handling
  const onMouseDown = useCallback((e: React.MouseEvent) => {
    if (e.button === 1 || (e.button === 0 && e.altKey)) {
      isPanning.current = true;
      lastMouse.current = { x: e.clientX, y: e.clientY };
      e.preventDefault();
    }
  }, []);

  const onMouseMove = useCallback((e: React.MouseEvent) => {
    if (isPanning.current) {
      const dx = e.clientX - lastMouse.current.x;
      const dy = e.clientY - lastMouse.current.y;
      setPan(panX + dx, panY + dy);
      lastMouse.current = { x: e.clientX, y: e.clientY };
    }
    if (hoveredStructureId !== null) {
      setTooltipPos({ x: e.clientX, y: e.clientY });
    }
  }, [panX, panY, setPan, hoveredStructureId]);

  const onMouseUp = useCallback(() => {
    isPanning.current = false;
  }, []);

  // Zoom handling
  const onWheel = useCallback((e: React.WheelEvent) => {
    e.preventDefault();
    const factor = e.deltaY > 0 ? 0.98 : 1.02;
    setZoom(zoomLevel * factor);
  }, [zoomLevel, setZoom]);

  // Displayed image size
  const scale = Math.pow(2, downsample);
  const imgW = section ? Math.ceil(section.width / scale) : 0;
  const imgH = section ? Math.ceil(section.height / scale) : 0;

  return (
    <div className={styles.container}>
      {/* View plane toggle */}
      <div className={styles.viewPlaneToggle}>
        {(['coronal', 'sagittal'] as ViewPlane[]).map((plane) => (
          <button
            key={plane}
            className={`${styles.planeBtn} ${viewPlane === plane ? styles.planeBtnActive : ''}`}
            onClick={() => setViewPlane(plane)}
          >
            {plane.charAt(0).toUpperCase() + plane.slice(1)}
          </button>
        ))}
      </div>
      {svgLoading && <div className={styles.loadingBar} />}
      {navigating && <div className={styles.searchingBanner}>Searching sections for region...</div>}
      <div
        ref={containerRef}
        className={styles.viewport}
        onMouseDown={onMouseDown}
        onMouseMove={onMouseMove}
        onMouseUp={onMouseUp}
        onMouseLeave={onMouseUp}
        onWheel={onWheel}
      >
        <div
          className={styles.canvas}
          style={{
            transform: `translate(${panX}px, ${panY}px) scale(${zoomLevel})`,
            transformOrigin: 'center center',
          }}
        >
          {section && (
            <>
              <img
                src={imageUrl}
                alt={`Section ${section.section_number}`}
                width={imgW}
                height={imgH}
                className={styles.sectionImage}
                onLoad={() => setImageLoaded(true)}
                draggable={false}
              />
              {svgRegions.length > 0 && (
                <SvgOverlay
                  regions={svgRegions}
                  viewBox={svgViewBox}
                  displayWidth={imgW}
                  displayHeight={imgH}
                />
              )}
            </>
          )}
        </div>

        {/* Zoom indicator */}
        <div className={styles.zoomIndicator}>
          <button onClick={() => setZoom(zoomLevel + 0.05)} className={styles.zoomBtn}>+</button>
          <span className={styles.zoomLabel}>{Math.round(zoomLevel * 100)}%</span>
          <button onClick={() => setZoom(zoomLevel - 0.05)} className={styles.zoomBtn}>-</button>
          <button onClick={resetView} className={styles.zoomBtn} title="Reset view">&#8634;</button>
        </div>
      </div>

      {tooltipPos && hoveredStructureId !== null && (
        <RegionTooltip structureId={hoveredStructureId} x={tooltipPos.x} y={tooltipPos.y} />
      )}

      <SectionStrip />
    </div>
  );
}
