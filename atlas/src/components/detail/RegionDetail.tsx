import { useEffect, useState, useCallback, useMemo, useRef, type ReactNode } from 'react';
import ReactMarkdown from 'react-markdown';
import { useAtlasStore } from '../../store/atlasStore';
import { findNode } from '../../utils/treeUtils';
import {
  fetchAllRegions,
  fetchRegionSummaries,
  fetchRegionStatus,
  fetchChunkSource,
  generateSummary,
  fetchBatchStatus,
} from '../../api/cortexmap';
import type { CortexmapRegion, OntologyNode, RegionSummary, RegionStatus, SummarySource } from '../../types';
import styles from './RegionDetail.module.css';

interface BatchStatusData {
  batch_id?: string;
  status: string;
  message?: string;
  error?: string;
  expected_tasks?: number;
  completed_tasks?: number | null;
}

const ACTIVE_STATUSES = new Set(['FetchQueued', 'Fetching', 'LlmQueued', 'Processing']);

export function RegionDetail() {
  const { selectedStructureId, ontology, cortexmapRegionMap, cortexmapLoaded, setCortexmapRegions } = useAtlasStore();
  const [summaries, setSummaries] = useState<RegionSummary[]>([]);
  const [status, setStatus] = useState<RegionStatus | null>(null);
  const [summaryLoading, setSummaryLoading] = useState(false);
  const [summaryError, setSummaryError] = useState<string | null>(null);

  // Generate button state
  const [isGenerating, setIsGenerating] = useState(false);
  const [generateError, setGenerateError] = useState<string | null>(null);
  const [batchId, setBatchId] = useState<string | null>(null);
  const [batchStatus, setBatchStatus] = useState<BatchStatusData | null>(null);
  const batchPollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const statusPollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Load cortexmap regions on first mount
  useEffect(() => {
    if (cortexmapLoaded) return;
    fetchAllRegions()
      .then((regions) => setCortexmapRegions(regions))
      .catch((err) => console.error('Failed to fetch cortexmap regions:', err));
  }, [cortexmapLoaded, setCortexmapRegions]);

  const ontologyNode = selectedStructureId !== null && ontology
    ? findNode(ontology, selectedStructureId)
    : null;

  const cortexmapRegion = selectedStructureId !== null
    ? cortexmapRegionMap.get(selectedStructureId) || null
    : null;

  // Fetch summaries + status when a cortexmap region is selected
  useEffect(() => {
    if (!cortexmapRegion) {
      setSummaries([]);
      setStatus(null);
      setSummaryError(null);
      return;
    }

    let cancelled = false;
    setSummaryLoading(true);
    setSummaryError(null);

    const uuid = cortexmapRegion.id;

    Promise.all([
      fetchRegionSummaries(uuid).catch((err) => { console.error('Failed to fetch summaries:', err); return [] as RegionSummary[]; }),
      fetchRegionStatus(uuid).catch((err) => { console.error('Failed to fetch region status:', err); return null; }),
    ]).then(([sums, st]) => {
      if (cancelled) return;
      setSummaries(sums);
      setStatus(st);
      setSummaryLoading(false);
    }).catch((err) => {
      if (cancelled) return;
      setSummaryError('Failed to load summary data');
      setSummaryLoading(false);
    });

    return () => { cancelled = true; };
  }, [cortexmapRegion?.id]);

  // Reset generate state when region changes
  useEffect(() => {
    setIsGenerating(false);
    setGenerateError(null);
    setBatchId(null);
    setBatchStatus(null);
    if (batchPollRef.current) { clearInterval(batchPollRef.current); batchPollRef.current = null; }
    if (statusPollRef.current) { clearInterval(statusPollRef.current); statusPollRef.current = null; }
  }, [cortexmapRegion?.id]);

  // Auto-refresh status while pipeline is active
  useEffect(() => {
    if (!cortexmapRegion || !status || !ACTIVE_STATUSES.has(status.status)) {
      if (statusPollRef.current) { clearInterval(statusPollRef.current); statusPollRef.current = null; }
      return;
    }
    if (statusPollRef.current) return; // already polling
    statusPollRef.current = setInterval(async () => {
      try {
        const [newStatus, newSummaries] = await Promise.all([
          fetchRegionStatus(cortexmapRegion.id),
          fetchRegionSummaries(cortexmapRegion.id),
        ]);
        setStatus(newStatus);
        setSummaries(newSummaries);
      } catch { /* silent */ }
    }, 3000);
    return () => { if (statusPollRef.current) { clearInterval(statusPollRef.current); statusPollRef.current = null; } };
  }, [status?.status, cortexmapRegion?.id]);

  // Poll batch status while a batch is in flight
  useEffect(() => {
    if (!batchId || !cortexmapRegion) return;
    if (batchPollRef.current) clearInterval(batchPollRef.current);
    batchPollRef.current = setInterval(async () => {
      try {
        const data = await fetchBatchStatus(batchId) as BatchStatusData;
        setBatchStatus(data);
        if (data.status === 'Completed' || data.status === 'Failed') {
          if (batchPollRef.current) { clearInterval(batchPollRef.current); batchPollRef.current = null; }
          setBatchId(null);
          setBatchStatus(null);
          const [newStatus, newSummaries] = await Promise.all([
            fetchRegionStatus(cortexmapRegion.id),
            fetchRegionSummaries(cortexmapRegion.id),
          ]);
          setStatus(newStatus);
          setSummaries(newSummaries);
        }
      } catch { /* silent */ }
    }, 2000);
    return () => { if (batchPollRef.current) { clearInterval(batchPollRef.current); batchPollRef.current = null; } };
  }, [batchId, cortexmapRegion?.id]);

  const handleGenerate = useCallback(async () => {
    if (!cortexmapRegion) return;
    setIsGenerating(true);
    setGenerateError(null);
    try {
      const data = await generateSummary(cortexmapRegion.id) as { batch_id?: string };
      if (data?.batch_id) {
        setBatchId(data.batch_id);
        setBatchStatus({ status: 'Queued', message: 'Summary generation started' });
      }
      const newStatus = await fetchRegionStatus(cortexmapRegion.id);
      setStatus(newStatus);
    } catch {
      setGenerateError('Failed to start summary generation. Please try again.');
    } finally {
      setIsGenerating(false);
    }
  }, [cortexmapRegion]);

  if (selectedStructureId === null) {
    return (
      <div className={styles.container}>
        <div className={styles.placeholder}>
          <div className={styles.placeholderIcon}>{'\u{1F9E0}'}</div>
          <div className={styles.placeholderText}>Select a region to view details</div>
          <div className={styles.placeholderHint}>Click on the atlas or tree</div>
        </div>
      </div>
    );
  }

  return (
    <div className={styles.container}>
      {/* Header */}
      <div className={styles.header}>
        {ontologyNode && (
          <>
            <div className={styles.colorBanner} style={{ backgroundColor: ontologyNode.c ? `#${ontologyNode.c}` : '#475569' }} />
            <h2 className={styles.name}>{ontologyNode.n}</h2>
            <div className={styles.meta}>
              <span className={styles.acronym}>{ontologyNode.a}</span>
              <span className={styles.id}>ID: {ontologyNode.id}</span>
            </div>
          </>
        )}
        {!ontologyNode && cortexmapRegion && (
          <>
            <h2 className={styles.name}>{cortexmapRegion.name}</h2>
            <div className={styles.meta}>
              <span className={styles.acronym}>{cortexmapRegion.acronym || 'N/A'}</span>
              <span className={styles.id}>ID: {cortexmapRegion.region_id}</span>
            </div>
          </>
        )}
      </div>

      {/* Allen info */}
      {ontologyNode && (
        <CollapsibleSection title="Allen Ontology">
          <div className={styles.infoGrid}>
            <div className={styles.infoLabel}>Graph Order</div>
            <div className={styles.infoValue}>{ontologyNode.o ?? 'N/A'}</div>
            <div className={styles.infoLabel}>ST Level</div>
            <div className={styles.infoValue}>{ontologyNode.l ?? 'N/A'}</div>
            <div className={styles.infoLabel}>Children</div>
            <div className={styles.infoValue}>{ontologyNode.ch.length}</div>
          </div>
        </CollapsibleSection>
      )}

      {/* CortexMap info */}
      {cortexmapRegion && (
        <CollapsibleSection title="CortexMap Region">
          <div className={styles.infoGrid}>
            <div className={styles.infoLabel}>UUID</div>
            <div className={styles.infoValue} style={{ fontSize: 10 }}>{cortexmapRegion.id}</div>
            <div className={styles.infoLabel}>Structure Order</div>
            <div className={styles.infoValue}>{cortexmapRegion.structure_order ?? 'N/A'}</div>
            <div className={styles.infoLabel}>Parent</div>
            <div className={styles.infoValue}>{cortexmapRegion.parent_acronym || 'N/A'}</div>
          </div>
          {status && (
            <div className={styles.statusRow}>
              <span className={styles.statusLabel}>Status</span>
              <span className={styles.statusBadge} data-status={status.status}>{status.status}</span>
            </div>
          )}
        </CollapsibleSection>
      )}

      {!cortexmapRegion && cortexmapLoaded && (
        <CollapsibleSection title="CortexMap">
          <p className={styles.noSummary}>This region is not tracked in CortexMap.</p>
        </CollapsibleSection>
      )}

      {/* Summary section */}
      {cortexmapRegion && (
        <CollapsibleSection title="Summary" defaultOpen>
          {/* Generate / Regenerate button row */}
          <div className={styles.generateRow}>
            {(!status || status.status === 'NotStarted') ? (
              <button
                className={styles.generateBtn}
                onClick={handleGenerate}
                disabled={isGenerating}
                type="button"
              >
                {isGenerating ? (
                  <><SpinnerIcon /> Initiating…</>
                ) : (
                  <>
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
                      <circle cx="12" cy="12" r="10" /><line x1="12" y1="8" x2="12" y2="16" /><line x1="8" y1="12" x2="16" y2="12" />
                    </svg>
                    Start Summary Generation
                  </>
                )}
              </button>
            ) : (
              <button
                className={`${styles.generateBtn} ${styles.generateBtnSecondary}`}
                onClick={handleGenerate}
                disabled={isGenerating || !!batchId || (status ? ACTIVE_STATUSES.has(status.status) : false)}
                type="button"
              >
                {isGenerating ? (
                  <><SpinnerIcon /> Starting…</>
                ) : (
                  <>
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
                      <polyline points="23 4 23 10 17 10" /><polyline points="1 20 1 14 7 14" />
                      <path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15" />
                    </svg>
                    Regenerate Summary
                  </>
                )}
              </button>
            )}
          </div>

          {generateError && <div className={styles.errorText} style={{ marginTop: 8 }}>{generateError}</div>}

          {/* Batch progress panel */}
          {batchStatus && batchId && (
            <div className={styles.batchProgress}>
              <div className={styles.batchHeader}>
                <SpinnerIcon className={styles.batchSpinner} />
                <span>Generation in progress</span>
              </div>
              <div className={styles.batchMeta}>
                <span className={styles.batchLabel}>Status:</span>
                <span>{batchStatus.status}</span>
              </div>
              {batchStatus.message && (
                <div className={styles.batchMeta}>
                  <span className={styles.batchLabel}>Info:</span>
                  <span>{batchStatus.message}</span>
                </div>
              )}
              {(batchStatus.expected_tasks ?? 0) > 0 && (
                <>
                  <div className={styles.batchMeta}>
                    <span className={styles.batchLabel}>Tasks:</span>
                    <span>{batchStatus.completed_tasks ?? 0} / {batchStatus.expected_tasks} fetched</span>
                  </div>
                  <div className={styles.progressBarTrack}>
                    <div
                      className={styles.progressBarFill}
                      style={{
                        width: `${Math.min(100, Math.round(((batchStatus.completed_tasks ?? 0) / batchStatus.expected_tasks!) * 100))}%`,
                      }}
                    />
                  </div>
                </>
              )}
              {batchStatus.error && (
                <div className={`${styles.errorText}`} style={{ marginTop: 4 }}>{batchStatus.error}</div>
              )}
            </div>
          )}

          {summaryLoading && <div className={styles.loadingText} style={{ marginTop: 10 }}>Loading summary…</div>}
          {summaryError && <div className={styles.errorText} style={{ marginTop: 8 }}>{summaryError}</div>}

          {/* In-progress placeholder */}
          {!summaryLoading && summaries.length === 0 && status && ACTIVE_STATUSES.has(status.status) && (
            <div className={styles.inProgressPlaceholder}>
              <SpinnerIcon className={styles.inProgressSpinner} />
              <span>Generating summary — {STATUS_LABELS[status.status] ?? status.status}</span>
            </div>
          )}

          {/* No summary yet */}
          {!summaryLoading && summaries.length === 0 && (!status || (!ACTIVE_STATUSES.has(status.status))) && !batchId && (
            <p className={styles.noSummary}>No summary generated yet.</p>
          )}

          {!summaryLoading && summaries.length > 0 && (
            <div className={styles.summariesList}>
              {summaries.map((s, i) => (
                <SummaryCard key={s.batch_id + '-' + i} summary={s} isLatest={i === 0} />
              ))}
            </div>
          )}
        </CollapsibleSection>
      )}

    </div>
  );
}

// ─── Helpers ─────────────────────────────────────────────────────────

const STATUS_LABELS: Record<string, string> = {
  FetchQueued: 'Fetch queued',
  Fetching: 'Fetching papers',
  LlmQueued: 'LLM queued',
  Processing: 'Generating summary',
};

function SpinnerIcon({ className }: { className?: string }) {
  return (
    <svg
      className={className}
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.5"
      style={{ animation: 'rd-spin 0.8s linear infinite', flexShrink: 0 }}
      aria-hidden="true"
    >
      <path d="M12 2v4M12 18v4M4.93 4.93l2.83 2.83M16.24 16.24l2.83 2.83M2 12h4M18 12h4M4.93 19.07l2.83-2.83M16.24 7.76l2.83-2.83" />
    </svg>
  );
}

// ─── Collapsible Section ─────────────────────────────────────────────

function CollapsibleSection({ title, defaultOpen = true, children }: { title: string; defaultOpen?: boolean; children: ReactNode }) {
  const [open, setOpen] = useState(defaultOpen);

  return (
    <div className={styles.section}>
      <button
        className={styles.sectionHeader}
        onClick={() => setOpen((v) => !v)}
        type="button"
      >
        <h3 className={styles.sectionTitle}>{title}</h3>
        <span className={`${styles.collapseChevron} ${open ? styles.chevronOpen : ''}`}>{'\u25B8'}</span>
      </button>
      {open && <div className={styles.sectionBody}>{children}</div>}
    </div>
  );
}

// ─── Summary Card with Chunk Citations ───────────────────────────────

interface ChunkInfo {
  chunk_id: string;
  pmc_id: string | null;
  source_query: string | null;
}

function SummaryCard({ summary, isLatest }: { summary: RegionSummary; isLatest?: boolean }) {
  const [chunkMap, setChunkMap] = useState<Record<string, ChunkInfo>>({});

  useEffect(() => {
    const loadChunks = async () => {
      // 1. Build initial map from summary.sources
      const initial: Record<string, ChunkInfo> = {};
      if (summary.sources) {
        for (const src of summary.sources) {
          initial[src.chunk_id] = {
            chunk_id: src.chunk_id,
            pmc_id: src.pmc_id,
            source_query: src.source_query,
          };
        }
      }

      // 2. Extract [chunk:UUID] references from markdown
      const pattern = /\[chunk:([a-f0-9-]+)\]/g;
      const allIds = new Set<string>();
      let m: RegExpExecArray | null;
      while ((m = pattern.exec(summary.summary)) !== null) {
        allIds.add(m[1]);
      }

      // 3. Fetch any missing chunk sources
      const missing = Array.from(allIds).filter((id) => !initial[id]);
      if (missing.length > 0) {
        const results = await Promise.all(
          missing.map(async (id) => {
            const data = await fetchChunkSource(id);
            return { id, data };
          })
        );
        for (const { id, data } of results) {
          if (data) {
            initial[id] = {
              chunk_id: id,
              pmc_id: data.source_pmc_id,
              source_query: data.source_query,
            };
          }
        }
      }

      setChunkMap(initial);
    };
    loadChunks();
  }, [summary]);

  const uniquePapers = useMemo(() => {
    const pmcIds = new Set<string>();
    Object.values(chunkMap).forEach((c) => {
      if (c.pmc_id) pmcIds.add(c.pmc_id);
    });
    return pmcIds.size;
  }, [chunkMap]);

  return (
    <div className={styles.summaryItem}>
      <div className={styles.summaryText}>
        <MarkdownWithChunks content={summary.summary} chunkMap={chunkMap} />
      </div>
      <div className={styles.summaryMeta}>
        {isLatest && <span className={styles.latestBadge}>Latest</span>}
        <span className={styles.summaryDate}>
          {new Date(summary.created_at).toLocaleDateString('en-US', {
            year: 'numeric', month: 'short', day: 'numeric',
            hour: '2-digit', minute: '2-digit',
          })}
        </span>
        {summary.sources && summary.sources.length > 0 && (
          <span className={styles.sourceCount}>
            {summary.sources.length} chunks{uniquePapers > 0 ? ` / ${uniquePapers} papers` : ''}
          </span>
        )}
      </div>
    </div>
  );
}

// ─── Markdown renderer that replaces [chunk:UUID] with citation bubbles ──

function MarkdownWithChunks({ content, chunkMap }: { content: string; chunkMap: Record<string, ChunkInfo> }) {
  // Replace [chunk:UUID] with a unique marker that survives markdown parsing
  const processed = content.replace(/\[chunk:([a-f0-9-]+)\]/g, '§CHUNK§$1§');

  const components = {
    p: ({ children, ...props }: any) => <p {...props}>{processChildren(children, chunkMap)}</p>,
    li: ({ children, ...props }: any) => <li {...props}>{processChildren(children, chunkMap)}</li>,
    strong: ({ children, ...props }: any) => <strong {...props}>{processChildren(children, chunkMap)}</strong>,
    em: ({ children, ...props }: any) => <em {...props}>{processChildren(children, chunkMap)}</em>,
    h1: ({ children, ...props }: any) => <h1 {...props}>{processChildren(children, chunkMap)}</h1>,
    h2: ({ children, ...props }: any) => <h2 {...props}>{processChildren(children, chunkMap)}</h2>,
    h3: ({ children, ...props }: any) => <h3 {...props}>{processChildren(children, chunkMap)}</h3>,
    h4: ({ children, ...props }: any) => <h4 {...props}>{processChildren(children, chunkMap)}</h4>,
  };

  return <ReactMarkdown components={components}>{processed}</ReactMarkdown>;
}

function processChildren(children: ReactNode, chunkMap: Record<string, ChunkInfo>): ReactNode {
  if (typeof children === 'string') {
    if (!children.includes('§CHUNK§')) return children;
    const segments = children.split(/(§CHUNK§[a-f0-9-]+§)/);
    const parts: ReactNode[] = [];
    for (let i = 0; i < segments.length; i++) {
      const seg = segments[i];
      if (seg.startsWith('§CHUNK§') && seg.endsWith('§')) {
        const chunkId = seg.slice(7, -1);
        parts.push(<ChunkBubble key={`c-${chunkId}-${i}`} chunkId={chunkId} source={chunkMap[chunkId] || null} />);
      } else if (seg) {
        parts.push(seg);
      }
    }
    return parts.length > 1 ? <>{parts}</> : children;
  }
  if (Array.isArray(children)) {
    return children.map((child, idx) => {
      if (typeof child === 'string') return <span key={idx}>{processChildren(child, chunkMap)}</span>;
      return child;
    });
  }
  return children;
}

// ─── Clickable citation bubble ──────────────────────────────────────

function ChunkBubble({ chunkId, source }: { chunkId: string; source: ChunkInfo | null }) {
  const [showTooltip, setShowTooltip] = useState(false);
  const pmcUrl = source?.pmc_id ? `https://www.ncbi.nlm.nih.gov/pmc/articles/${source.pmc_id}/` : null;
  const displayText = source?.pmc_id || chunkId.substring(0, 8);

  return (
    <span
      className={styles.chunkWrapper}
      onMouseEnter={() => setShowTooltip(true)}
      onMouseLeave={() => setShowTooltip(false)}
    >
      {pmcUrl ? (
        <a href={pmcUrl} target="_blank" rel="noopener noreferrer" className={styles.chunkLink}>
          {displayText}
          <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" style={{ marginLeft: 2 }}>
            <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
            <polyline points="15 3 21 3 21 9" />
            <line x1="10" y1="14" x2="21" y2="3" />
          </svg>
        </a>
      ) : (
        <span className={styles.chunkPlain}>{displayText}</span>
      )}
      {showTooltip && (
        <span className={styles.chunkTooltip}>
          <span className={styles.tooltipRow}>
            <span className={styles.tooltipLabel}>PMC:</span>
            <span>{source?.pmc_id || 'N/A'}</span>
          </span>
          <span className={styles.tooltipRow}>
            <span className={styles.tooltipLabel}>Chunk:</span>
            <span className={styles.tooltipChunkId}>{chunkId}</span>
          </span>
          {source?.source_query && (
            <span className={styles.tooltipRow}>
              <span className={styles.tooltipLabel}>Query:</span>
              <span>{decodeURIComponent(source.source_query.replace(/\+/g, ' '))}</span>
            </span>
          )}
        </span>
      )}
    </span>
  );
}
