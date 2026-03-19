import { useState, useMemo, useEffect, useRef, useCallback } from 'react';
import axios from 'axios';
import ReactMarkdown from 'react-markdown';
import { Search, Filter, Loader2 } from 'lucide-react';
import { API_BASE_URL, logger } from '../config';
import './RegionList.css';

function RegionList({ regions, onRegionSelect }) {
  const [searchTerm, setSearchTerm] = useState('');
  const [sortBy, setSortBy] = useState('name');
  const [searchResults, setSearchResults] = useState(null);
  const [searchLoading, setSearchLoading] = useState(false);
  const debounceRef = useRef(null);

  // Client-side filtering (instant, used when query is short)
  const filteredAndSortedRegions = useMemo(() => {
    let filtered = regions.filter(region =>
      region.name.toLowerCase().includes(searchTerm.toLowerCase()) ||
      region.acronym.toLowerCase().includes(searchTerm.toLowerCase())
    );

    return filtered.sort((a, b) => {
      if (sortBy === 'name') {
        return a.name.localeCompare(b.name);
      } else if (sortBy === 'acronym') {
        return a.acronym.localeCompare(b.acronym);
      } else if (sortBy === 'structure_order') {
        return a.structure_order - b.structure_order;
      }
      return 0;
    });
  }, [regions, searchTerm, sortBy]);

  // Debounced backend search (fires when query is 2+ chars)
  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);

    if (searchTerm.trim().length < 2) {
      setSearchResults(null);
      setSearchLoading(false);
      return;
    }

    setSearchLoading(true);
    debounceRef.current = setTimeout(async () => {
      try {
        const url = `${API_BASE_URL}/search`;
        logger.api('POST', url, { query: searchTerm });
        const response = await axios.post(url, { query: searchTerm.trim() });
        logger.apiSuccess('POST', url, response.data);
        setSearchResults(response.data);
      } catch (err) {
        logger.apiError('POST', `${API_BASE_URL}/search`, err);
        setSearchResults(null);
      } finally {
        setSearchLoading(false);
      }
    }, 300);

    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, [searchTerm]);

  const handleResultClick = useCallback((result) => {
    // Map search result back to the full pre-loaded region object
    const fullRegion = regions.find(r => r.id === result.region_id);
    if (fullRegion) {
      onRegionSelect(fullRegion);
    } else {
      // Fallback: construct a minimal region object so RegionDetail still works
      onRegionSelect({
        id: result.region_id,
        region_id: result.region_numeric_id,
        name: result.name,
        acronym: result.acronym || '',
        color: null,
        parent_acronym: null,
        structure_order: 0,
      });
    }
  }, [regions, onRegionSelect]);

  const rgbToHex = (color) => {
    if (!color) return '#6366f1';
    const { red, green, blue } = color;
    return `#${((1 << 24) + (red << 16) + (green << 8) + blue).toString(16).slice(1)}`;
  };

  const showBackendResults = searchTerm.trim().length >= 2 && searchResults !== null;

  return (
    <div className="region-list-container">
      <div className="controls-bar">
        <div className="search-box">
          <Search size={20} className="search-icon" />
          <input
            type="text"
            placeholder="Search by name, acronym, or describe what you're looking for..."
            value={searchTerm}
            onChange={(e) => setSearchTerm(e.target.value)}
            className="search-input"
          />
          {searchLoading && (
            <Loader2 size={18} className="search-spinner spinning" />
          )}
        </div>

        {!showBackendResults && (
          <div className="filter-box">
            <Filter size={20} className="filter-icon" />
            <select
              value={sortBy}
              onChange={(e) => setSortBy(e.target.value)}
              className="sort-select"
            >
              <option value="name">Sort by Name</option>
              <option value="acronym">Sort by Acronym</option>
              <option value="structure_order">Sort by Structure Order</option>
            </select>
          </div>
        )}
      </div>

      {searchLoading && searchTerm.trim().length >= 2 ? (
        <div className="search-loading">
          <Loader2 size={32} className="spinning" />
          <p>Searching across regions and summaries&hellip;</p>
        </div>
      ) : showBackendResults ? (
        <>
          <div className="region-stats">
            <p>
              Found {searchResults.total_found} result{searchResults.total_found !== 1 ? 's' : ''} for
              &nbsp;&ldquo;{searchResults.query}&rdquo;
              {searchResults.total_found > searchResults.results.length && (
                <span className="stats-detail">
                  &nbsp;(showing top {searchResults.results.length})
                </span>
              )}
            </p>
          </div>

          <div className="search-results-list">
            {searchResults.results.map((result) => {
              const fullRegion = regions.find(r => r.id === result.region_id);
              return (
                <div
                  key={result.region_id}
                  className="search-result-card"
                  onClick={() => handleResultClick(result)}
                >
                  <div
                    className="search-result-color-bar"
                    style={{ backgroundColor: fullRegion ? rgbToHex(fullRegion.color) : '#6366f1' }}
                  />
                  <div className="search-result-content">
                    <div className="search-result-header">
                      <div className="search-result-title">
                        <h3 className="region-name">{result.name}</h3>
                        {result.acronym && (
                          <span className="region-acronym">{result.acronym}</span>
                        )}
                      </div>
                      <div className="search-result-meta">
                        <span className={`match-badge match-badge-${result.match_source}`}>
                          {result.match_source}
                        </span>
                        <span className="rank-badge">
                          {Math.round(result.rank * 100)}%
                        </span>
                      </div>
                    </div>
                    {result.summary_snippet && (
                      <div className="search-result-snippet">
                        <HighlightedMarkdown text={result.summary_snippet} keyword={searchResults.query} />
                      </div>
                    )}
                  </div>
                </div>
              );
            })}
          </div>

          {searchResults.results.length === 0 && (
            <div className="no-results">
              <p>No regions found matching &ldquo;{searchTerm}&rdquo;</p>
            </div>
          )}
        </>
      ) : (
        <>
          <div className="region-stats">
            <p>Showing {filteredAndSortedRegions.length} of {regions.length} regions</p>
          </div>

          <div className="regions-grid">
            {filteredAndSortedRegions.map((region) => (
              <div
                key={region.id}
                className="region-card"
                onClick={() => onRegionSelect(region)}
              >
                <div
                  className="region-color-bar"
                  style={{ backgroundColor: rgbToHex(region.color) }}
                />
                <div className="region-card-content">
                  <div className="region-header">
                    <h3 className="region-name">{region.name}</h3>
                    <span className="region-acronym">{region.acronym}</span>
                  </div>

                  <div className="region-metadata">
                    <div className="metadata-item">
                      <span className="metadata-label">Region ID:</span>
                      <span className="metadata-value">{region.region_id}</span>
                    </div>

                    {region.parent_acronym && (
                      <div className="metadata-item">
                        <span className="metadata-label">Parent:</span>
                        <span className="metadata-value">{region.parent_acronym}</span>
                      </div>
                    )}

                    <div className="metadata-item">
                      <span className="metadata-label">Order:</span>
                      <span className="metadata-value">{region.structure_order}</span>
                    </div>
                  </div>
                </div>
              </div>
            ))}
          </div>

          {filteredAndSortedRegions.length === 0 && (
            <div className="no-results">
              <p>No regions found matching &ldquo;{searchTerm}&rdquo;</p>
            </div>
          )}
        </>
      )}
    </div>
  );
}

/**
 * Renders markdown text with all occurrences of `keyword` highlighted in yellow.
 * Uses ReactMarkdown for parsing, then injects <mark> tags into text nodes.
 */
function HighlightedMarkdown({ text, keyword }) {
  const highlightText = useCallback((children) => {
    if (!keyword) return children;

    if (typeof children === 'string') {
      const regex = new RegExp(`(${keyword.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')})`, 'gi');
      const parts = children.split(regex);
      if (parts.length === 1) return children;
      return parts.map((part, i) =>
        regex.test(part)
          ? <mark key={i} className="search-highlight">{part}</mark>
          : part
      );
    }

    if (Array.isArray(children)) {
      return children.map((child, i) =>
        typeof child === 'string'
          ? <span key={i}>{highlightText(child)}</span>
          : child
      );
    }

    return children;
  }, [keyword]);

  const components = {
    p: ({ children, ...props }) => <p {...props}>{highlightText(children)}</p>,
    li: ({ children, ...props }) => <li {...props}>{highlightText(children)}</li>,
    strong: ({ children, ...props }) => <strong {...props}>{highlightText(children)}</strong>,
    em: ({ children, ...props }) => <em {...props}>{highlightText(children)}</em>,
    h1: ({ children, ...props }) => <h1 {...props}>{highlightText(children)}</h1>,
    h2: ({ children, ...props }) => <h2 {...props}>{highlightText(children)}</h2>,
    h3: ({ children, ...props }) => <h3 {...props}>{highlightText(children)}</h3>,
    h4: ({ children, ...props }) => <h4 {...props}>{highlightText(children)}</h4>,
  };

  // Strip [chunk:...] references -- they don't make sense in a preview
  const cleaned = text.replace(/\[chunk:[a-f0-9-]+\]/g, '');

  return <ReactMarkdown components={components}>{cleaned}</ReactMarkdown>;
}

export default RegionList;
