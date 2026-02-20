import { useState, useEffect } from 'react';
import axios from 'axios';
import ReactMarkdown from 'react-markdown';
import { ExternalLink, ChevronDown, ChevronUp } from 'lucide-react';
import { API_BASE_URL, logger } from '../config';
import './SummaryDisplay.css';

function SummaryDisplay({ summaries, formatDate }) {
  const [showAllSummaries, setShowAllSummaries] = useState(false);
  
  if (!summaries || summaries.length === 0) {
    return null;
  }

  // Filter out summaries with empty or null summary strings
  const validSummaries = summaries.filter(s => s.summary && s.summary.trim().length > 0);

  if (validSummaries.length === 0) {
    return null;
  }

  // Latest summary first
  const latestSummary = validSummaries[0];
  const olderSummaries = validSummaries.slice(1);

  return (
    <div className="summary-display">
      <LatestSummary summary={latestSummary} formatDate={formatDate} />
      
      {olderSummaries.length > 0 && (
        <div className="older-summaries-section">
          <button
            className="toggle-summaries-btn"
            onClick={() => setShowAllSummaries(!showAllSummaries)}
          >
            {showAllSummaries ? (
              <>
                <ChevronUp size={16} />
                Hide {olderSummaries.length} Previous {olderSummaries.length === 1 ? 'Summary' : 'Summaries'}
              </>
            ) : (
              <>
                <ChevronDown size={16} />
                Show {olderSummaries.length} Previous {olderSummaries.length === 1 ? 'Summary' : 'Summaries'}
              </>
            )}
          </button>

          {showAllSummaries && (
            <div className="older-summaries-list">
              {olderSummaries.map((summary, index) => (
                <OlderSummary
                  key={index}
                  summary={summary}
                  index={index + 2}
                  formatDate={formatDate}
                />
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function LatestSummary({ summary, formatDate }) {
  const [chunkMap, setChunkMap] = useState({});
  const [loadingChunks, setLoadingChunks] = useState(false);

  useEffect(() => {
    const loadChunkData = async () => {
      // Build initial chunk map from sources
      const initialMap = {};
      if (summary.sources) {
        summary.sources.forEach(source => {
          initialMap[source.chunk_id] = source;
        });
      }

      // Extract all chunk IDs from the summary markdown
      const chunkIdPattern = /\[chunk:([a-f0-9-]+)\]/g;
      const allChunkIds = new Set();
      let match;
      while ((match = chunkIdPattern.exec(summary.summary)) !== null) {
        allChunkIds.add(match[1]);
      }

      // Find chunk IDs that are missing source data
      const missingChunkIds = Array.from(allChunkIds).filter(id => !initialMap[id]);

      if (missingChunkIds.length > 0) {
        logger.info(`Found ${missingChunkIds.length} chunks without source data, fetching...`);
        setLoadingChunks(true);

        try {
          // Make batch async requests for all missing chunks
          const chunkRequests = missingChunkIds.map(chunkId => 
            axios.get(`${API_BASE_URL}/chunks/${chunkId}/source`)
              .then(response => ({ chunkId, data: response.data }))
              .catch(err => {
                logger.apiError('GET', `${API_BASE_URL}/chunks/${chunkId}/source`, err);
                return { chunkId, data: null };
              })
          );

          const results = await Promise.all(chunkRequests);
          logger.apiSuccess('Batch chunk fetch', 'Multiple chunk sources', { 
            count: results.length,
            successful: results.filter(r => r.data).length
          });

          // Merge fetched data into chunk map
          const updatedMap = { ...initialMap };
          results.forEach(({ chunkId, data }) => {
            if (data) {
              logger.debug(`Chunk ${chunkId}: PMC ${data.source_pmc_id}`);
              updatedMap[chunkId] = {
                chunk_id: chunkId,
                pmc_id: data.source_pmc_id,  // API returns source_pmc_id
                source_query: data.source_query,
                uid: data.source_uid
              };
            } else {
              logger.debug(`Chunk ${chunkId}: No data received`);
            }
          });

          logger.info(`Chunk map now has ${Object.keys(updatedMap).length} entries`);
          setChunkMap(updatedMap);
        } catch (err) {
          logger.error('Error fetching chunk sources:', err);
          setChunkMap(initialMap);
        } finally {
          setLoadingChunks(false);
        }
      } else {
        setChunkMap(initialMap);
      }
    };

    loadChunkData();
  }, [summary]);

  return (
    <div className="latest-summary">
      <div className="summary-header-latest">
        <div className="summary-badge latest">Latest</div>
        <span className="summary-date">{formatDate(summary.created_at)}</span>
      </div>
      
      <div className="summary-content-latest">
        {loadingChunks && (
          <div className="chunk-loading-notice">Loading chunk references...</div>
        )}
        <MarkdownWithChunks content={summary.summary} chunkMap={chunkMap} />
      </div>

      {summary.sources && summary.sources.length > 0 && (
        <div className="sources-summary">
          <h4>Sources</h4>
          <p>{summary.sources.length} chunks from {new Set(summary.sources.map(s => s.pmc_id)).size} papers</p>
        </div>
      )}
    </div>
  );
}

function OlderSummary({ summary, index, formatDate }) {
  const [chunkMap, setChunkMap] = useState({});
  const [loadingChunks, setLoadingChunks] = useState(false);

  useEffect(() => {
    const loadChunkData = async () => {
      // Build initial chunk map from sources
      const initialMap = {};
      if (summary.sources) {
        summary.sources.forEach(source => {
          initialMap[source.chunk_id] = source;
        });
      }

      // Extract all chunk IDs from the summary markdown
      const chunkIdPattern = /\[chunk:([a-f0-9-]+)\]/g;
      const allChunkIds = new Set();
      let match;
      while ((match = chunkIdPattern.exec(summary.summary)) !== null) {
        allChunkIds.add(match[1]);
      }

      // Find chunk IDs that are missing source data
      const missingChunkIds = Array.from(allChunkIds).filter(id => !initialMap[id]);

      if (missingChunkIds.length > 0) {
        logger.info(`Found ${missingChunkIds.length} chunks without source data in older summary, fetching...`);
        setLoadingChunks(true);

        try {
          // Make batch async requests for all missing chunks
          const chunkRequests = missingChunkIds.map(chunkId => 
            axios.get(`${API_BASE_URL}/chunks/${chunkId}/source`)
              .then(response => ({ chunkId, data: response.data }))
              .catch(err => {
                logger.apiError('GET', `${API_BASE_URL}/chunks/${chunkId}/source`, err);
                return { chunkId, data: null };
              })
          );

          const results = await Promise.all(chunkRequests);

          // Merge fetched data into chunk map
          const updatedMap = { ...initialMap };
          results.forEach(({ chunkId, data }) => {
            if (data) {
              updatedMap[chunkId] = {
                chunk_id: chunkId,
                pmc_id: data.source_pmc_id,  // API returns source_pmc_id
                source_query: data.source_query,
                uid: data.source_uid
              };
            }
          });

          setChunkMap(updatedMap);
        } catch (err) {
          logger.error('Error fetching chunk sources:', err);
          setChunkMap(initialMap);
        } finally {
          setLoadingChunks(false);
        }
      } else {
        setChunkMap(initialMap);
      }
    };

    loadChunkData();
  }, [summary]);

  return (
    <div className="older-summary">
      <div className="summary-header-older">
        <span className="summary-number">#{index}</span>
        <span className="summary-date">{formatDate(summary.created_at)}</span>
      </div>
      
      <div className="summary-content-older">
        {loadingChunks && (
          <div className="chunk-loading-notice">Loading chunk references...</div>
        )}
        <MarkdownWithChunks content={summary.summary} chunkMap={chunkMap} />
      </div>
    </div>
  );
}

function MarkdownWithChunks({ content, chunkMap }) {
  // Use a unique marker that won't be touched by markdown or HTML
  const processedContent = content.replace(/\[chunk:([a-f0-9-]+)\]/g, (match, chunkId) => {
    return `§§§CHUNK§${chunkId}§§§`;
  });

  const components = {
    // Process all text-containing elements
    p: ({ children, ...props }) => <p {...props}>{processChildren(children, chunkMap)}</p>,
    li: ({ children, ...props }) => <li {...props}>{processChildren(children, chunkMap)}</li>,
    strong: ({ children, ...props }) => <strong {...props}>{processChildren(children, chunkMap)}</strong>,
    em: ({ children, ...props }) => <em {...props}>{processChildren(children, chunkMap)}</em>,
    h1: ({ children, ...props }) => <h1 {...props}>{processChildren(children, chunkMap)}</h1>,
    h2: ({ children, ...props }) => <h2 {...props}>{processChildren(children, chunkMap)}</h2>,
    h3: ({ children, ...props }) => <h3 {...props}>{processChildren(children, chunkMap)}</h3>,
    h4: ({ children, ...props }) => <h4 {...props}>{processChildren(children, chunkMap)}</h4>,
    code: ({ children, ...props }) => <code {...props}>{processChildren(children, chunkMap)}</code>,
  };

  return (
    <div className="markdown-content">
      <ReactMarkdown components={components}>{processedContent}</ReactMarkdown>
    </div>
  );
}

function processChildren(children, chunkMap) {
  if (typeof children === 'string') {
    // Look for our chunk markers
    if (!children.includes('§§§CHUNK§')) {
      return children;
    }

    const parts = [];
    const segments = children.split(/(§§§CHUNK§[a-f0-9-]+§§§)/);
    
    for (let i = 0; i < segments.length; i++) {
      const segment = segments[i];
      
      if (segment.startsWith('§§§CHUNK§') && segment.endsWith('§§§')) {
        // Extract chunk ID from marker
        const chunkId = segment.slice(9, -3); // Remove §§§CHUNK§ prefix and §§§ suffix
        const source = chunkMap[chunkId];
        parts.push(<ChunkBubble key={`chunk-${chunkId}-${i}`} chunkId={chunkId} source={source} />);
      } else if (segment) {
        // Regular text
        parts.push(segment);
      }
    }
    
    return parts.length > 1 ? parts : children;
  }

  if (Array.isArray(children)) {
    return children.map((child, idx) => {
      if (typeof child === 'string') {
        const processed = processChildren(child, chunkMap);
        // If processed is an array, return it with unique keys
        if (Array.isArray(processed)) {
          return processed.map((item, subIdx) => 
            typeof item === 'string' ? <span key={`text-${idx}-${subIdx}`}>{item}</span> : item
          );
        }
        return processed;
      }
      return child;
    });
  }

  return children;
}

function ChunkBubble({ chunkId, source }) {
  const [showTooltip, setShowTooltip] = useState(false);
  const pmcUrl = source?.pmc_id ? `https://www.ncbi.nlm.nih.gov/pmc/articles/${source.pmc_id}/` : null;
  const displayText = source?.pmc_id || chunkId.substring(0, 8);

  return (
    <span
      className="chunk-bubble-wrapper"
      onMouseEnter={() => setShowTooltip(true)}
      onMouseLeave={() => setShowTooltip(false)}
    >
      {source?.pmc_id ? (
        <a href={pmcUrl} target="_blank" rel="noopener noreferrer" className="chunk-bubble chunk-link">
          {displayText}
          <ExternalLink size={10} />
        </a>
      ) : (
        <span className="chunk-bubble chunk-plain">{displayText}</span>
      )}
      
      {showTooltip && source && (
        <div className="chunk-tooltip">
          <div className="tooltip-row">
            <span className="tooltip-label">PMC:</span>
            <span className="tooltip-value">
              {source.pmc_id || 'N/A'}
              {pmcUrl && (
                <a href={pmcUrl} target="_blank" rel="noopener noreferrer" className="tooltip-link">
                  <ExternalLink size={11} />
                </a>
              )}
            </span>
          </div>
          <div className="tooltip-row">
            <span className="tooltip-label">Chunk:</span>
            <span className="tooltip-value tooltip-chunk-id">{chunkId}</span>
          </div>
          {source.source_query && (
            <div className="tooltip-row tooltip-query-row">
              <span className="tooltip-label">Query:</span>
              <span className="tooltip-value tooltip-query">
                {decodeURIComponent(source.source_query.replace(/\+/g, ' '))}
              </span>
            </div>
          )}
        </div>
      )}
    </span>
  );
}

export default SummaryDisplay;
