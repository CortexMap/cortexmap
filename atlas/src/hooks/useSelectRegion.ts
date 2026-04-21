import { useCallback } from 'react';
import { useSearchParams } from 'react-router-dom';

/**
 * Returns a function that selects a brain region by navigating to
 * `?region=<allen_structure_id>`. The URL is the single source of truth --
 * the store is updated by the URL -> store effect in App.tsx when the URL
 * changes (including on browser back/forward navigation).
 *
 * Pass `null` to clear the selection.
 *
 * Call this from any UI event handler (tree click, SVG click, search
 * result, 3D mesh click, etc.) instead of calling `selectStructure`
 * on the store directly. That way the browser owns the history stack
 * and back/forward always work correctly.
 */
export function useSelectRegion() {
  const [searchParams, setSearchParams] = useSearchParams();

  return useCallback(
    (structureId: number | null) => {
      const next = new URLSearchParams(searchParams);
      if (structureId == null) next.delete('region');
      else next.set('region', String(structureId));
      setSearchParams(next, { replace: false });
    },
    [searchParams, setSearchParams]
  );
}
