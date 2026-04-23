import React, { useEffect, useState, useCallback, useMemo, useRef, type ReactNode, isValidElement, cloneElement } from 'react';
import ReactMarkdown from 'react-markdown';
import type { Components } from 'react-markdown';
import { useAtlasStore } from '../../store/atlasStore';
import { useAtlasSearchRevealContext } from '../../hooks/useSelectRegion';
import { findNode } from '../../utils/treeUtils';
import {
  fetchAllRegions,
  fetchRegionSummaries,
  fetchRegionStatus,
  fetchChunkSource,
  generateSummary,
  fetchBatchStatus,
} from '../../api/cortexmap';
import type { CortexmapRegion, OntologyNode, RegionSummary, RegionStatus, SummarySource, SummaryEvalScores, AtlasSearchRevealContext } from '../../types';
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
const SEARCH_HIGHLIGHT_DURATION_MS = 3200;

export function RegionDetail() {
  const { selectedStructureId, ontology, cortexmapRegionMap, cortexmapLoaded, setCortexmapRegions } = useAtlasStore();
  const { context: searchRevealContext, clearContext: clearSearchRevealContext } = useAtlasSearchRevealContext();
  const [summaries, setSummaries] = useState<RegionSummary[]>([]);
  const [status, setStatus] = useState<RegionStatus | null>(null);
  const [summaryLoading, setSummaryLoading] = useState(false);
  const [summaryError, setSummaryError] = useState<string | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);

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
      <div className={styles.container} ref={containerRef}>
        <div className={styles.placeholder}>
          <div className={styles.placeholderIcon}>{'\u{1F9E0}'}</div>
          <div className={styles.placeholderText}>Select a region to view details</div>
          <div className={styles.placeholderHint}>Click on the atlas or tree</div>
        </div>
      </div>
    );
  }

  return (
    <div className={styles.container} ref={containerRef}>
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
                <SummaryCard
                  key={s.batch_id + '-' + i}
                  summary={s}
                  isLatest={i === 0}
                  searchRevealContext={searchRevealContext}
                  clearSearchRevealContext={clearSearchRevealContext}
                  detailContainerRef={containerRef}
                />
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

interface SummaryBlockMatch {
  index: number;
  score: number;
}

function normalizeMatchText(value: string): string {
  return value
    .replace(/\[chunk:[a-f0-9-]+\]/gi, ' ')
    .replace(/[#>*_`~\-]+/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
    .toLowerCase();
}

function escapeRegex(value: string) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function scoreSummaryBlockMatch(
  blockText: string,
  context: AtlasSearchRevealContext | null,
  isLatest: boolean
): number {
  if (!context) return isLatest ? 1 : 0;

  let score = isLatest ? 1 : 0;
  const normalizedBlock = normalizeMatchText(blockText);
  const normalizedSnippet = normalizeMatchText(context.summary_snippet ?? '').replace(/…/g, ' ').trim();
  const normalizedQuery = normalizeMatchText(context.query);

  if (normalizedSnippet) {
    if (normalizedBlock.includes(normalizedSnippet)) {
      score += 120;
    } else {
      const snippetTokens = normalizedSnippet.split(' ').filter((token) => token.length >= 4);
      const matchedSnippetTokens = snippetTokens.filter((token) => normalizedBlock.includes(token));
      if (matchedSnippetTokens.length > 0) {
        score += matchedSnippetTokens.length * 12;
        score += matchedSnippetTokens.length === snippetTokens.length ? 24 : 0;
      }
    }
  }

  if (normalizedQuery) {
    const tokens = normalizedQuery.split(' ').filter((token) => token.length >= 2);
    for (const token of tokens) {
      if (normalizedBlock.includes(token)) score += 8;
    }
  }

  if (context.match_source === 'summary') score += 6;
  if (context.match_source === 'name' || context.match_source === 'acronym') score += isLatest ? 4 : 0;

  return score;
}

function findBestSummaryBlockMatch(
  blocks: string[],
  context: AtlasSearchRevealContext | null,
  isLatest: boolean
): SummaryBlockMatch {
  if (blocks.length === 0) return { index: 0, score: 0 };

  let best: SummaryBlockMatch = { index: 0, score: Number.NEGATIVE_INFINITY };
  blocks.forEach((block, index) => {
    const score = scoreSummaryBlockMatch(block, context, isLatest && index === 0);
    if (score > best.score) {
      best = { index, score };
    }
  });

  return best;
}

function SummaryCard({
  summary,
  isLatest,
  searchRevealContext,
  clearSearchRevealContext,
  detailContainerRef,
}: {
  summary: RegionSummary;
  isLatest?: boolean;
  searchRevealContext: AtlasSearchRevealContext | null;
  clearSearchRevealContext: () => void;
  detailContainerRef: React.RefObject<HTMLDivElement | null>;
}) {
  const [chunkMap, setChunkMap] = useState<Record<string, ChunkInfo>>({});
  const [activeBlockIndex, setActiveBlockIndex] = useState<number | null>(null);
  const blockRefs = useRef<Array<HTMLElement | null>>([]);

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

  useEffect(() => {
    blockRefs.current = [];
  }, [summary.summary]);

  useEffect(() => {
    if (!isLatest) return;

    const shouldReveal = !!searchRevealContext || isLatest;
    if (!shouldReveal) return;

    let cancelled = false;
    let timer: number | null = null;

    const revealMatchedBlock = (attempt = 0) => {
      if (cancelled) return;

      const renderedBlocks = blockRefs.current.map((node) => normalizeMatchText(node?.innerText ?? ''));
      const hasRenderableBlocks = renderedBlocks.length > 0 && renderedBlocks.some((block) => block);
      const container = detailContainerRef.current;

      if (!container || !hasRenderableBlocks) {
        if (attempt < 10) {
          timer = window.setTimeout(() => revealMatchedBlock(attempt + 1), 120);
        }
        return;
      }

      const { index, score } = findBestSummaryBlockMatch(renderedBlocks, searchRevealContext, true);
      const resolvedIndex = score > 0 ? index : 0;
      const target = blockRefs.current[resolvedIndex];

      if (!target) {
        if (attempt < 10) {
          timer = window.setTimeout(() => revealMatchedBlock(attempt + 1), 120);
        }
        return;
      }

      setActiveBlockIndex(resolvedIndex);
      scrollBlockIntoDetailPane(container, target);

      timer = window.setTimeout(() => {
        setActiveBlockIndex((current) => (current === resolvedIndex ? null : current));
        if (searchRevealContext) clearSearchRevealContext();
      }, SEARCH_HIGHLIGHT_DURATION_MS);
    };

    timer = window.setTimeout(() => revealMatchedBlock(0), 80);

    return () => {
      cancelled = true;
      if (timer !== null) {
        window.clearTimeout(timer);
      }
    };
  }, [clearSearchRevealContext, detailContainerRef, isLatest, searchRevealContext, summary.summary]);


  return (
    <div className={styles.summaryItem}>
      <div className={styles.summaryText}>
        <BlockAwareMarkdown
          content={summary.summary}
          chunkMap={chunkMap}
          highlightQuery={searchRevealContext?.query ?? ''}
          activeBlockIndex={activeBlockIndex}
          onBlockRef={(index, node) => {
            blockRefs.current[index] = node;
          }}
        />
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
      {summary.eval_scores && <EvalScoresBar scores={summary.eval_scores} />}
    </div>
  );
}

function scrollBlockIntoDetailPane(container: HTMLDivElement, target: HTMLElement) {
  const containerRect = container.getBoundingClientRect();
  const targetRect = target.getBoundingClientRect();
  const currentScrollTop = container.scrollTop;
  const targetTop = targetRect.top - containerRect.top + currentScrollTop;
  const targetBottom = targetRect.bottom - containerRect.top + currentScrollTop;
  const topOffset = Math.max(24, container.clientHeight * 0.18);
  const bottomOffset = Math.max(24, container.clientHeight * 0.12);
  const visibleTop = currentScrollTop + topOffset;
  const visibleBottom = currentScrollTop + container.clientHeight - bottomOffset;

  let nextScrollTop = currentScrollTop;

  if (targetTop < visibleTop) {
    nextScrollTop = Math.max(0, targetTop - topOffset);
  } else if (targetBottom > visibleBottom) {
    nextScrollTop = Math.max(0, targetBottom - container.clientHeight + bottomOffset);
  }

  if (Math.abs(nextScrollTop - currentScrollTop) < 2) {
    return;
  }

  container.scrollTo({
    top: nextScrollTop,
    behavior: 'smooth',
  });
}

// ─── Eval scores strip (rendered at the bottom of each scored summary) ──

type MetricGroup = 'truth' | 'structure' | 'style';

const METRIC_DISPLAY: { key: string; label: string; group: MetricGroup; invert?: boolean; description: string }[] = [
  // ─── HEADLINE: truth & evidence ─────────────────────────────────────
  // These are the metrics we optimise the pipeline for. Higher correlation
  // with actual factual quality. Hallucination and groundedness are r=−0.90.
  {
    key: 'claim_groundedness',
    label: 'Groundedness',
    group: 'truth',
    description: 'Groundedness judge (gpt-4o-mini): extracts atomic factual claims, retrieves the top-k most-similar source chunks, asks the judge whether each claim is supported. Score = fraction rated "supported".',
  },
  {
    key: 'hallucination_rate',
    label: 'Hallucination',
    group: 'truth',
    invert: true,
    description: 'Inverse of groundedness: the fraction of atomic claims the judge rated unsupported or partial. Low = good.',
  },
  {
    key: 'citation_scope',
    label: 'Cite Scope',
    group: 'truth',
    description: 'Citation check (no LLM): of the valid UUIDs, fraction that belong to this summary\u2019s own retrieval corpus (not leaked from a different summary).',
  },
  {
    key: 'citation_validity',
    label: 'Cite Validity',
    group: 'truth',
    description: 'Citation check (no LLM): of the chunk UUIDs referenced, fraction that resolve to a real row in brain_region_embeddings. Catches orphan/fabricated UUIDs.',
  },
  {
    key: 'citation_support',
    label: 'Cite Support',
    group: 'truth',
    description: 'Citation judge (LLM, opt-in): of the valid in-scope citations, fraction where the cited chunk text actually supports the adjacent claim. The true "citation correctness" check.',
  },
  {
    key: 'citation_presence',
    label: 'Cite Presence',
    group: 'truth',
    description: 'Citation check (no LLM): fraction of factual claims that include at least one [chunk:UUID] marker attributing the source. Measures how often the writer bothered to cite at all.',
  },
  // ─── STRUCTURE: free, mechanical checks ─────────────────────────────
  {
    key: 'section_completeness',
    label: 'Completeness',
    group: 'structure',
    description: 'Structural check (no LLM): fraction of required markdown sections present — Overview, Anatomy & Connectivity, Function, Clinical Relevance.',
  },
  {
    key: 'length_in_range',
    label: 'Length',
    group: 'structure',
    description: 'Structural check (no LLM): binary score confirming the summary word count falls within an acceptable window (not too short or bloated).',
  },
  {
    key: 'acronym_mention',
    label: 'Acronyms',
    group: 'structure',
    description: 'Structural check (no LLM): verifies the region\u2019s acronym (e.g. "IPN") appears at least once in the summary body.',
  },
  {
    key: 'no_placeholder_text',
    label: 'No Placeholders',
    group: 'structure',
    description: 'Structural check (no LLM): scans for LLM failure strings like "I cannot", "insufficient information", [TODO], etc. 0 if any found, 1 otherwise.',
  },
  // ─── STYLE: rubric judge — anti-correlated with groundedness (r=−0.08) ───
  // Kept visible as a context-free style signal but de-emphasised in the
  // overall score so we don't reward confident-sounding fabrication.
  {
    key: 'rubric_relevance',
    label: 'Relevance',
    group: 'style',
    description: 'Rubric judge (gpt-4o): does the summary actually describe the named region rather than drifting to neighbouring or parent structures?',
  },
  {
    key: 'rubric_coherence',
    label: 'Coherence',
    group: 'style',
    description: 'Rubric judge (gpt-4o): is the prose well-organised, internally consistent, and free of contradiction or redundancy?',
  },
  {
    key: 'rubric_specificity',
    label: 'Specificity',
    group: 'style',
    description: 'Rubric judge (gpt-4o): does it contain concrete neuroanatomical detail (cell types, layers, connections) rather than generic filler?',
  },
  {
    key: 'rubric_clinical_utility',
    label: 'Utility',
    group: 'style',
    description: 'Rubric judge (gpt-4o): would a clinician or neuroscientist find the summary actionable — named pathologies, functional roles, diagnostic relevance?',
  },
  {
    key: 'rubric_terminology',
    label: 'Terminology',
    group: 'style',
    description: 'Rubric judge (gpt-4o): does it use correct, standard neuroanatomical terminology (canonical Latin/Greek names, standard pathway labels)?',
  },
];

function scoreColor(score: number, invert: boolean): string {
  // For "invert" metrics (hallucination), a low value is good.
  const good = invert ? 1 - score : score;
  if (good >= 0.8) return '#15803d'; // green
  if (good >= 0.5) return '#b45309'; // amber
  return '#dc2626';                  // red
}

function EvalScoresBar({ scores }: { scores: SummaryEvalScores }) {
  const [infoOpen, setInfoOpen] = useState(false);

  const entries = METRIC_DISPLAY
    .map((m) => ({ ...m, value: scores.scores[m.key] }))
    .filter((m) => typeof m.value === 'number');

  if (entries.length === 0) return null;

  // Overall score: mean of TRUTH metrics only. Style metrics are visible
  // but excluded from the headline because they are anti-correlated with
  // groundedness (r = -0.08 across 2.5k summaries) and reward fluent
  // fabrication. Structure metrics are excluded because they are mostly
  // 1.0 by construction once basic checks pass.
  const truthEntries = entries.filter((m) => m.group === 'truth');
  const headlineEntries = truthEntries.length > 0 ? truthEntries : entries;
  const overall = headlineEntries.reduce(
    (acc, m) => acc + (m.invert ? 1 - (m.value as number) : (m.value as number)),
    0,
  ) / headlineEntries.length;

  const groupEntries = (g: MetricGroup) => entries.filter((m) => m.group === g);
  const truthGroup = groupEntries('truth');
  const structureGroup = groupEntries('structure');
  const styleGroup = groupEntries('style');

  const renderGroup = (label: string, items: typeof entries) => {
    if (items.length === 0) return null;
    return (
      <div className={styles.evalGroup}>
        <div className={styles.evalGroupLabel}>{label}</div>
        <div className={styles.evalGrid}>
          {items.map((m) => {
            const color = scoreColor(m.value as number, !!m.invert);
            const pct = Math.round(((m.value as number)) * 100);
            return (
              <div
                key={m.key}
                className={styles.evalMetric}
                title={`${m.label}: ${(m.value as number).toFixed(3)}${scores.judge_models[m.key] ? ` \u2014 judge: ${scores.judge_models[m.key]}` : ''}\n\n${m.description}`}
              >
                <span className={styles.evalLabel}>{m.label}</span>
                <span className={styles.evalValue} style={{ color }}>{pct}%</span>
                <span className={styles.evalTrack}>
                  <span
                    className={styles.evalFill}
                    style={{ width: `${(m.value as number) * 100}%`, backgroundColor: color }}
                  />
                </span>
              </div>
            );
          })}
        </div>
      </div>
    );
  };

  return (
    <div className={styles.evalBar} title={`Eval version: ${scores.eval_version}`}>
      <div className={styles.evalHeader}>
        <span className={styles.evalTitle}>Evaluation</span>
        <button
          type="button"
          className={styles.evalInfoBtn}
          onClick={(e) => { e.stopPropagation(); setInfoOpen((v) => !v); }}
          aria-expanded={infoOpen}
          aria-label="What do these metrics mean?"
          title="What do these metrics mean?"
        >
          i
        </button>
        <span className={styles.evalOverall} style={{ color: scoreColor(overall, false) }}>
          {(overall * 100).toFixed(0)}%
        </span>
        <span className={styles.evalVersion}>{scores.eval_version}</span>
      </div>

      {infoOpen && (
        <div className={styles.evalInfoPanel}>
          <div className={styles.evalInfoHeader}>
            <span>Metric definitions</span>
            <button
              type="button"
              className={styles.evalInfoClose}
              onClick={() => setInfoOpen(false)}
              aria-label="Close"
            >
              {'\u00D7'}
            </button>
          </div>
          <dl className={styles.evalInfoList}>
            {METRIC_DISPLAY.map((m) => (
              <div key={m.key} className={styles.evalInfoItem}>
                <dt className={styles.evalInfoLabel}>{m.label}</dt>
                <dd className={styles.evalInfoDesc}>{m.description}</dd>
              </div>
            ))}
          </dl>
          <div className={styles.evalInfoFooter}>
            Overall = mean of TRUTH metrics only (hallucination flipped so higher = better).
            Style metrics are displayed for context but excluded from the
            headline score because they are anti-correlated with groundedness.
            Green &#8805; 80%, amber 50\u201379%, red &lt; 50%.
          </div>
        </div>
      )}

      {renderGroup('Truth & evidence', truthGroup)}
      {renderGroup('Structure', structureGroup)}
      {renderGroup('Style', styleGroup)}
    </div>
  );
}

// ─── Markdown renderer that replaces [chunk:UUID] with citation bubbles ──

function BlockAwareMarkdown({
  content,
  chunkMap,
  highlightQuery,
  activeBlockIndex,
  onBlockRef,
}: {
  content: string;
  chunkMap: Record<string, ChunkInfo>;
  highlightQuery: string;
  activeBlockIndex: number | null;
  onBlockRef: (index: number, node: HTMLElement | null) => void;
}) {
  const processed = content.replace(/\[chunk:([a-f0-9-]+)\]/g, '§CHUNK§$1§');
  let blockCounter = -1;

  const wrapBlock = (tagName: keyof HTMLElementTagNameMap, children: ReactNode) => {
    blockCounter += 1;
    const index = blockCounter;
    const className = `${styles.summaryBlock}${activeBlockIndex === index ? ` ${styles.summaryBlockActive}` : ''}`;
    return React.createElement(
      tagName,
      {
        className,
        ref: (node: HTMLElement | null) => onBlockRef(index, node),
        'data-block-index': index,
      },
      renderHighlightedChildren(children, chunkMap, highlightQuery)
    );
  };

  const components: Components = {
    p: ({ children }) => wrapBlock('p', children),
    li: ({ children }) => wrapBlock('li', children),
    h1: ({ children }) => wrapBlock('h1', children),
    h2: ({ children }) => wrapBlock('h2', children),
    h3: ({ children }) => wrapBlock('h3', children),
    h4: ({ children }) => wrapBlock('h4', children),
    strong: ({ children, ...props }) => <strong {...props}>{renderHighlightedChildren(children, chunkMap, highlightQuery)}</strong>,
    em: ({ children, ...props }) => <em {...props}>{renderHighlightedChildren(children, chunkMap, highlightQuery)}</em>,
    blockquote: ({ children }) => wrapBlock('blockquote', children),
  };

  return <ReactMarkdown components={components}>{processed}</ReactMarkdown>;
}

function renderHighlightedChildren(
  children: ReactNode,
  chunkMap: Record<string, ChunkInfo>,
  highlightQuery: string
): ReactNode {
  const withChunks = processChildren(children, chunkMap);
  return applyHighlightsToNode(withChunks, highlightQuery);
}

function applyHighlightsToNode(node: ReactNode, highlightQuery: string): ReactNode {
  if (!highlightQuery.trim()) return node;

  if (typeof node === 'string') {
    const pattern = new RegExp(`(${escapeRegex(highlightQuery.trim())})`, 'gi');
    const parts = node.split(pattern);
    if (parts.length === 1) return node;
    return parts.map((part, index) =>
      pattern.test(part) ? (
        <mark key={`${part}-${index}`} className={styles.inlineHighlight}>{part}</mark>
      ) : (
        part
      )
    );
  }

  if (Array.isArray(node)) {
    return node.map((child, index) => <React.Fragment key={index}>{applyHighlightsToNode(child, highlightQuery)}</React.Fragment>);
  }

  if (isValidElement<{ children?: ReactNode }>(node)) {
    return cloneElement(node, undefined, applyHighlightsToNode(node.props.children, highlightQuery));
  }

  return node;
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
