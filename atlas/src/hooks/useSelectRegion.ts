import { useCallback, useEffect, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import type { AtlasSearchRevealContext } from '../types';

const SEARCH_CONTEXT_PARAM = 'searchContext';

function encodeSearchContext(context: AtlasSearchRevealContext): string {
  return window.btoa(encodeURIComponent(JSON.stringify(context)));
}

function decodeSearchContext(value: string | null): AtlasSearchRevealContext | null {
  if (!value) return null;
  try {
    return JSON.parse(decodeURIComponent(window.atob(value))) as AtlasSearchRevealContext;
  } catch {
    return null;
  }
}

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
    (structureId: number | null, options?: { searchContext?: AtlasSearchRevealContext | null }) => {
      const next = new URLSearchParams(searchParams);
      if (structureId == null) {
        next.delete('region');
        next.delete(SEARCH_CONTEXT_PARAM);
      } else {
        next.set('region', String(structureId));
        if (options?.searchContext) {
          next.set(SEARCH_CONTEXT_PARAM, encodeSearchContext(options.searchContext));
        } else {
          next.delete(SEARCH_CONTEXT_PARAM);
        }
      }
      setSearchParams(next, { replace: false });
    },
    [searchParams, setSearchParams]
  );
}

export function useAtlasSearchRevealContext() {
  const [searchParams, setSearchParams] = useSearchParams();
  const [context, setContext] = useState<AtlasSearchRevealContext | null>(() =>
    decodeSearchContext(searchParams.get(SEARCH_CONTEXT_PARAM))
  );

  useEffect(() => {
    setContext(decodeSearchContext(searchParams.get(SEARCH_CONTEXT_PARAM)));
  }, [searchParams]);

  const clearContext = useCallback(() => {
    const next = new URLSearchParams(searchParams);
    next.delete(SEARCH_CONTEXT_PARAM);
    setSearchParams(next, { replace: true });
    setContext(null);
  }, [searchParams, setSearchParams]);

  return { context, clearContext };
}
