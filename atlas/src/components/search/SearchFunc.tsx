import React, { useState, useEffect, useRef, useCallback } from 'react';
import axios from 'axios';
import ReactMarkdown from 'react-markdown';
import type { Components } from 'react-markdown';
import { useAtlasStore } from '../../store/atlasStore';
import './SearchFunc.css';

const API_BASE = import.meta.env.VITE_API_BASE_URL || 'https://capstone.ssdd.dev/orch/api';

interface SearchResultItem {
  region_id: string;
  region_numeric_id: number;
  name: string;
  acronym: string | null;
  summary_snippet: string | null;
  match_source: string;
  rank: number;
}

interface SearchResponse {
  query: string;
  results: SearchResultItem[];
  total_found: number;
}

export function SearchFunc() {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<SearchResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);

  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const overlayRef = useRef<HTMLDivElement>(null);

  const { selectStructure, cortexmapRegionMap } = useAtlasStore();

  // Focus input when popup opens; reset state when closed
  useEffect(() => {
    if (open) {
      setTimeout(() => inputRef.current?.focus(), 50);
    } else {
      setQuery('');
      setResults(null);
      setLoading(false);
      setActiveIndex(-1);
    }
  }, [open]);

  // Debounced search — fires for queries of 2+ characters
  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);

    if (query.trim().length < 2) {
      setResults(null);
      setLoading(false);
      setActiveIndex(-1);
      return;
    }

    setLoading(true);
    debounceRef.current = setTimeout(async () => {
      try {
        const { data } = await axios.post<SearchResponse>(`${API_BASE}/search`, {
          query: query.trim(),
        });
        setResults(data);
        setActiveIndex(-1);
      } catch {
        setResults(null);
      } finally {
        setLoading(false);
      }
    }, 300);

    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, [query]);

  // Scroll the active list item into view
  useEffect(() => {
    if (activeIndex >= 0 && listRef.current) {
      const item = listRef.current.children[activeIndex] as HTMLElement | undefined;
      item?.scrollIntoView({ block: 'nearest' });
    }
  }, [activeIndex]);

  // Close when clicking outside the modal
  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (overlayRef.current && !overlayRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  // Global keyboard shortcut: Ctrl/Cmd+K to open, Escape to close
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        setOpen(false);
      } else if (e.key === 'k' && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        setOpen((prev) => !prev);
      }
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, []);

  const handleSelect = useCallback(
    (item: SearchResultItem) => {
      // Resolve the Allen structure ID from the cortexmap region map
      const cortexmapEntry = Array.from(cortexmapRegionMap.values()).find(
        (r) => r.id === item.region_id
      );
      const structureId = cortexmapEntry?.region_id ?? item.region_numeric_id;
      selectStructure(structureId);
      setOpen(false);
    },
    [cortexmapRegionMap, selectStructure]
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLInputElement>) => {
      const items = results?.results ?? [];
      if (items.length === 0) return;

      if (e.key === 'ArrowDown') {
        e.preventDefault();
        setActiveIndex((i) => Math.min(i + 1, items.length - 1));
      } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        setActiveIndex((i) => Math.max(i - 1, 0));
      } else if (e.key === 'Enter') {
        e.preventDefault();
        const idx = activeIndex >= 0 ? activeIndex : 0;
        if (items[idx]) handleSelect(items[idx]);
      }
    },
    [results, activeIndex, handleSelect]
  );

  const hexFromColor = (
    color: { red: number; green: number; blue: number } | null | undefined
  ) => {
    if (!color) return '#7c3aed';
    return `#${((1 << 24) + (color.red << 16) + (color.green << 8) + color.blue)
      .toString(16)
      .slice(1)}`;
  };

  const hasResults = results !== null && results.results.length > 0;
  const noResults =
    results !== null &&
    results.results.length === 0 &&
    query.trim().length >= 2 &&
    !loading;

  return (
    <>
      {/* ── Trigger button ────────────────────────────────────── */}
      <button
        className="sf-trigger"
        onClick={() => setOpen(true)}
        title="Search regions (Ctrl+K)"
        aria-label="Search brain regions"
      >
        <svg
          className="sf-trigger-icon"
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2.3"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <circle cx="11" cy="11" r="8" />
          <line x1="21" y1="21" x2="16.65" y2="16.65" />
        </svg>
        <span className="sf-trigger-label">Search</span>
        <kbd className="sf-trigger-kbd">⌘K</kbd>
      </button>

      {/* ── Modal overlay ────────────────────────────────────── */}
      {open && (
        <dialog
          className="sf-modal"
          ref={overlayRef as React.RefObject<HTMLDialogElement>}
          open
          aria-label="Search brain regions"
        >
            {/* Input row */}
            <div className="sf-input-row">
              <svg
                className="sf-input-icon"
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2.2"
                strokeLinecap="round"
                strokeLinejoin="round"
                aria-hidden="true"
              >
                <circle cx="11" cy="11" r="8" />
                <line x1="21" y1="21" x2="16.65" y2="16.65" />
              </svg>
              <input
                ref={inputRef}
                className="sf-input"
                type="text"
                placeholder="Search by name, acronym, or describe a region…"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={handleKeyDown}
                autoComplete="off"
                spellCheck={false}
                aria-label="Search query"
                aria-autocomplete="list"
                aria-controls="sf-results-list"
              />
              {loading && (
                <svg
                  className="sf-spinner"
                  width="16"
                  height="16"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2.5"
                  aria-hidden="true"
                >
                  <path d="M12 2v4M12 18v4M4.93 4.93l2.83 2.83M16.24 16.24l2.83 2.83M2 12h4M18 12h4M4.93 19.07l2.83-2.83M16.24 7.76l2.83-2.83" />
                </svg>
              )}
            </div>

            {/* Results area */}
            {(hasResults || noResults) && (
              <div className="sf-results-container">
                {hasResults && (
                  <>
                    <div className="sf-results-header">
                      {results!.total_found} result
                      {results!.total_found !== 1 ? 's' : ''} for{' '}
                      <strong>&ldquo;{results!.query}&rdquo;</strong>
                      {results!.total_found > results!.results.length && (
                        <span className="sf-results-count">
                          {' '}
                          &mdash; showing top {results!.results.length}
                        </span>
                      )}
                    </div>

                    <ul
                      id="sf-results-list"
                      ref={listRef}
                      className="sf-list"
                      aria-label="Search results"
                    >
                      {results!.results.map((item, idx) => {
                        const cortexmapEntry = Array.from(cortexmapRegionMap.values()).find(
                          (r) => r.id === item.region_id
                        );
                        const hex = hexFromColor(cortexmapEntry?.color);
                        const isActive = idx === activeIndex;

                        return (
                          <li key={item.region_id} id={`sf-item-${idx}`} className="sf-item-wrapper">
                            <button
                              type="button"
                              className={`sf-item${isActive ? ' sf-item--active' : ''}`}
                              onMouseEnter={() => setActiveIndex(idx)}
                              onMouseLeave={() => setActiveIndex(-1)}
                              onClick={() => handleSelect(item)}
                            >
                              <span
                                className="sf-item-color"
                                style={{ background: hex }}
                                aria-hidden="true"
                              />
                              <span className="sf-item-body">
                                <span className="sf-item-title">
                                  <span className="sf-item-name">{item.name}</span>
                                  {item.acronym && (
                                    <span className="sf-item-acronym">{item.acronym}</span>
                                  )}
                                </span>
                                {item.summary_snippet && (
                                  <span className="sf-item-snippet">
                                    <SnippetMarkdown text={item.summary_snippet} />
                                  </span>
                                )}
                              </span>
                              <span className="sf-item-meta" aria-hidden="true">
                                <span className={`sf-badge sf-badge--${item.match_source}`}>
                                  {item.match_source}
                                </span>
                                <span className="sf-rank">{Math.round(item.rank * 100)}%</span>
                              </span>
                            </button>
                          </li>
                        );
                      })}
                    </ul>
                  </>
                )}

                {noResults && (
                  <div className="sf-empty">
                    No regions found for &ldquo;{query}&rdquo;
                  </div>
                )}
              </div>
            )}

            {/* Keyboard hint bar */}
            <div className="sf-hint-bar" aria-hidden="true">
              <span>
                <kbd>↑</kbd>
                <kbd>↓</kbd> navigate
              </span>
              <span>
                <kbd>↵</kbd> select
              </span>
              <span>
                <kbd>Esc</kbd> close
              </span>
            </div>
        </dialog>
      )}
    </>
  );
}

/**
 * Renders a summary snippet as inline Markdown.
 * [chunk:uuid] citation markers are stripped before rendering.
 * Block-level elements (p, li, headings) are mapped to inline spans
 * so the 2-line clamp on the parent .sf-item-snippet still applies.
 */
function SnippetMarkdown({ text }: { readonly text: string }) {
  const cleaned = text.replace(/\[chunk:[a-f0-9-]+\]/g, '').trim();

  const components: Components = {
    // Render block elements inline so the parent -webkit-line-clamp works
    p: ({ children }) => <span className="sf-md-p">{children}</span>,
    li: ({ children }) => <span className="sf-md-li">{children}</span>,
    ul: ({ children }) => <span className="sf-md-ul">{children}</span>,
    ol: ({ children }) => <span className="sf-md-ol">{children}</span>,
    h1: ({ children }) => <span className="sf-md-h">{children}</span>,
    h2: ({ children }) => <span className="sf-md-h">{children}</span>,
    h3: ({ children }) => <span className="sf-md-h">{children}</span>,
    h4: ({ children }) => <span className="sf-md-h">{children}</span>,
    // Preserve inline formatting
    strong: ({ children }) => <strong>{children}</strong>,
    em: ({ children }) => <em>{children}</em>,
    code: ({ children }) => <code className="sf-md-code">{children}</code>,
    // Strip links — keep text only (snippets shouldn't be clickable internally)
    a: ({ children }) => <span>{children}</span>,
  };

  return <ReactMarkdown components={components}>{cleaned}</ReactMarkdown>;
}
