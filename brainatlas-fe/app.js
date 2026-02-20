// Fetch data from API
const regionUrl = 'https://capstone.ssdd.dev/orch/api/regions';
let REGIONS_DATA = [];
const regionsById = new Map();
const regionsByRegionId = new Map();

function buildHierarchy(data) {
  const root = data.find(r => r.parent_region_id === null);
  const map = new Map();
  data.forEach(r => {
    map.set(r.region_id, { ...r, children: [] });
  });
  data.forEach(r => {
    if (r.parent_region_id !== null) {
      const parent = map.get(r.parent_region_id);
      if (parent) parent.children.push(map.get(r.region_id));
    }
  });
  return map.get(root.region_id);
}

// Initialize app with data from API
async function initializeApp() {
  try {
    const response = await fetch(regionUrl);
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    REGIONS_DATA = await response.json();
    
    // Update structure count in header
    const countEl = document.getElementById('structure-count');
    if (countEl) {
      const count = REGIONS_DATA.length.toLocaleString();
      countEl.textContent = `${count} structures`;
    }
    
    // Build maps
    REGIONS_DATA.forEach(r => {
      regionsById.set(r.id, r);
      regionsByRegionId.set(r.region_id, r);
    });
    
    // Build hierarchy and render
    const hierarchyData = buildHierarchy(REGIONS_DATA);
    renderVisualization(hierarchyData);
  } catch (error) {
    console.error('Failed to load regions data:', error);
    const countEl = document.getElementById('structure-count');
    if (countEl) countEl.textContent = 'Failed to load';
    document.getElementById('viz-container').innerHTML = `
      <div style="color: #ff6b6b; text-align: center; padding: 40px; font-family: 'Space Mono', monospace;">
        <h3>Failed to load brain regions data</h3>
        <p style="color: #888; font-size: 14px;">${error.message}</p>
      </div>
    `;
  }
}

function renderVisualization(hierarchyData) {

// Colors
function regionColor(d) {
  if (d.data && d.data.color) {
    const c = d.data.color;
    return `rgb(${c.red},${c.green},${c.blue})`;
  }
  return '#444';
}

function toRgb(c) {
  return `rgb(${c.red},${c.green},${c.blue})`;
}

// === SUNBURST ===
const container = document.getElementById('viz-container');
const W = container.clientWidth || 700;
const H = container.clientHeight || 700;
const R = Math.min(W, H) * 0.61; // Increased from 0.47 to 0.61 (30% larger)

const svg = d3.select('#viz-container')
  .append('svg')
  .attr('width', '100%')
  .attr('height', '100%')
  .style('display', 'block');

const g = svg.append('g')
  .attr('transform', `translate(${W/2},${H/2})`);

// Center circle click area
const centerCircle = g.append('circle')
  .attr('r', R * 0.12)
  .attr('fill', 'transparent')
  .attr('cursor', 'pointer')
  .style('z-index', 10);

const centerText = g.append('text')
  .attr('class', 'center-label')
  .attr('fill', '#4a5568')
  .attr('font-size', '11px')
  .text('zoom out');

const partition = d3.partition().size([2 * Math.PI, R]);
const rootHier = d3.hierarchy(hierarchyData)
  .sum(() => 1)
  .sort((a, b) => (a.data.structure_order || 0) - (b.data.structure_order || 0));

partition(rootHier);

const arc = d3.arc()
  .startAngle(d => d.x0)
  .endAngle(d => d.x1)
  .padAngle(0.003)
  .padRadius(R / 2)
  .innerRadius(d => Math.max(0, d.y0))
  .outerRadius(d => Math.max(0, d.y1 - 2));

let currentRoot = rootHier;
let modalRegion = null;

const path = g.append('g').attr('class', 'paths')
  .selectAll('path')
  .data(rootHier.descendants().filter(d => d.depth > 0 && d.depth <= 5))
  .enter().append('path')
  .attr('d', arc)
  .attr('fill', d => regionColor(d))
  .attr('stroke', '#0a0c10')
  .attr('stroke-width', 0.5)
  .attr('fill-opacity', 0.88)
  .style('cursor', 'pointer')
  .on('mouseover', function(event, d) {
    d3.select(this).attr('fill-opacity', 1).attr('stroke-width', 1.5).attr('stroke', '#fff');
    showTooltip(event, d);
    updatePanel(d.data);
  })
  .on('mousemove', function(event) {
    moveTooltip(event);
  })
  .on('mouseout', function(event, d) {
    d3.select(this).attr('fill-opacity', 0.88).attr('stroke-width', 0.5).attr('stroke', '#0a0c10');
    hideTooltip();
  })
  .on('click', function(event, d) {
    event.stopPropagation();
    // Single click: zoom if has children, otherwise show summary
    if (d.children && d.children.length > 0) {
      zoom(d);
    } else {
      showSummary(d.data);
    }
  })
  .on('dblclick', function(event, d) {
    event.stopPropagation();
    // Double-click: always show summary
    showSummary(d.data);
  });

centerCircle.on('click', () => {
  if (currentRoot !== rootHier) {
    zoom(currentRoot.parent || rootHier);
  }
});

// Zoom / filter
function zoom(targetNode) {
  currentRoot = targetNode;
  const t = svg.transition().duration(600).ease(d3.easeCubicInOut);

  // Update what's visible: show only descendants of targetNode within 4 levels
  const minDepth = targetNode.depth;
  const maxDepth = minDepth + 4;

  path
    .filter(d => {
      // visible if ancestor of target or descendant within range
      return isDescendantOrSelf(targetNode, d) && d.depth > minDepth && d.depth <= maxDepth;
    })
    .attr('pointer-events', 'all')
    .attr('visibility', 'visible');

  path
    .filter(d => {
      return !(isDescendantOrSelf(targetNode, d) && d.depth > minDepth && d.depth <= maxDepth);
    })
    .attr('pointer-events', 'none')
    .attr('visibility', 'hidden');

  // Re-partition for current root
  const newRoot = d3.hierarchy(targetNode.data)
    .sum(() => 1)
    .sort((a, b) => (a.data.structure_order || 0) - (b.data.structure_order || 0));
  partition(newRoot);

  // Remap positions
  const newByRegionId = new Map();
  newRoot.descendants().forEach(n => newByRegionId.set(n.data.region_id, n));

  path.transition(t)
    .attrTween('d', function(d) {
      const nn = newByRegionId.get(d.data.region_id);
      if (!nn) return () => '';
      const interp = d3.interpolate(
        { x0: d.x0, x1: d.x1, y0: d.y0, y1: d.y1 },
        { x0: nn.x0, x1: nn.x1, y0: nn.y0, y1: nn.y1 }
      );
      return t => {
        const v = interp(t);
        return arc({...d, ...v}) || '';
      };
    });

  updateBreadcrumb(targetNode);
  updatePanel(targetNode.data);
  centerText.text(targetNode === rootHier ? '' : '⬆ up');
}

function isDescendantOrSelf(ancestor, node) {
  let n = node;
  while (n) {
    if (n === ancestor) return true;
    n = n.parent;
  }
  return false;
}

function updateBreadcrumb(node) {
  const crumb = document.getElementById('breadcrumb');
  const ancestors = [];
  let n = node;
  while (n) { ancestors.unshift(n); n = n.parent; }
  crumb.innerHTML = ancestors.map((a, i) => {
    if (i < ancestors.length - 1) {
      return `<span onclick="zoomToNode(${a.data.region_id})">${a.data.name === 'root' ? '⌂' : a.data.name}</span><span class="breadcrumb-sep"> › </span>`;
    }
    return `<span style="color:var(--text)">${a.data.name}</span>`;
  }).join('');
}

window.zoomToNode = function(regionId) {
  const node = rootHier.descendants().find(d => d.data.region_id === regionId);
  if (node) zoom(node);
};

function updatePanel(data) {
  document.getElementById('panel-name').textContent = data.name;
  const swatch = document.getElementById('panel-swatch');
  if (data.color) {
    swatch.style.background = toRgb(data.color);
    swatch.style.display = 'block';
  }

  const node = rootHier.descendants().find(d => d.data.region_id === data.region_id);
  const children = node ? (node.children || []) : [];
  const parent = node ? node.parent : null;

  let html = `
    <div class="info-row">
      <div class="info-label">Acronym</div>
      <div class="info-value accent">${data.acronym || '—'}</div>
    </div>
    <div class="info-row">
      <div class="info-label">Region ID</div>
      <div class="info-value">${data.region_id}</div>
    </div>
    <div class="info-row">
      <div class="info-label">Parent</div>
      <div class="info-value">${data.parent_acronym || '—'}</div>
    </div>`;

  if (data.color) {
    html += `<div class="info-row">
      <div class="info-label">Color</div>
      <div class="info-value">rgb(${data.color.red}, ${data.color.green}, ${data.color.blue})</div>
    </div>`;
  }

  html += `<div class="info-row">
    <div class="info-label">Children</div>
    <div class="info-value">${children.length} sub-regions</div>
  </div>`;

  // Add View AI Summary button before sub-regions
  html += `<div class="section-title">AI Summary</div>
    <button onclick="showSummaryById(${data.region_id})" style="background:var(--bg);border:1px solid var(--border);color:var(--accent);font-family:'Space Mono',monospace;font-size:12px;padding:10px 16px;border-radius:4px;cursor:pointer;width:100%;text-align:left;transition:all 0.15s;font-weight:700;" onmouseover="this.style.borderColor='var(--accent)'" onmouseout="this.style.borderColor='var(--border)'">
      View AI Summary for "${data.acronym}" ↗
    </button>`;

  if (children.length > 0) {
    html += `<div class="section-title">Sub-regions</div><div class="children-list">`;
    children.slice(0, 20).forEach(c => {
      const cc = c.data.color ? toRgb(c.data.color) : '#444';
      html += `<div class="child-item" onclick="zoomToNodeAndPanel(${c.data.region_id})">
        <div class="swatch" style="background:${cc}"></div>
        <div class="name">${c.data.name}</div>
        <div class="acro">${c.data.acronym}</div>
      </div>`;
    });
    if (children.length > 20) html += `<div style="color:var(--text-dim);font-size:11px;padding:4px 10px;">…and ${children.length-20} more</div>`;
    html += '</div>';
  }

  document.getElementById('panel-body').innerHTML = html;
}

window.zoomToNodeAndPanel = function(regionId) {
  const node = rootHier.descendants().find(d => d.data.region_id === regionId);
  if (node) {
    zoom(node);
    updatePanel(node.data);
  }
};

// === TOOLTIP ===
const tooltip = document.getElementById('tooltip');
function showTooltip(event, d) {
  document.getElementById('tt-name').textContent = d.data.name;
  document.getElementById('tt-acro').textContent = d.data.acronym;
  const hasChildren = d.children && d.children.length > 0;
  document.getElementById('tt-hint').textContent = hasChildren
    ? 'Click to zoom in · Double-click for summary'
    : 'Click for AI summary';
  tooltip.style.display = 'block';
  moveTooltip(event);
}
function moveTooltip(event) {
  tooltip.style.left = (event.clientX + 14) + 'px';
  tooltip.style.top = (event.clientY - 10) + 'px';
}
function hideTooltip() {
  tooltip.style.display = 'none';
}

// === MODAL / SUMMARY ===
function showSummary(data) {
  modalRegion = data;
  const modal = document.getElementById('modal');
  const overlay = document.getElementById('modal-overlay');
  const color = data.color ? toRgb(data.color) : '#444';

  document.getElementById('modal-color').style.background = color;
  document.getElementById('modal-title').textContent = data.name;
  document.getElementById('modal-acro').textContent = data.acronym;
  document.getElementById('modal-id').textContent = `ID: ${data.region_id}`;
  document.getElementById('modal-body').innerHTML = `<div class="loading">Fetching summary<span class="loading-dot">.</span><span class="loading-dot">.</span><span class="loading-dot">.</span></div>`;

  overlay.classList.add('visible');
  fetchSummary(data);
}

window.showSummaryById = function(regionId) {
  const node = rootHier.descendants().find(d => d.data.region_id === regionId);
  if (node) showSummary(node.data);
};

document.getElementById('modal-close').onclick = () => {
  document.getElementById('modal-overlay').classList.remove('visible');
};
document.getElementById('modal-overlay').onclick = (e) => {
  if (e.target === document.getElementById('modal-overlay')) {
    document.getElementById('modal-overlay').classList.remove('visible');
  }
};
document.getElementById('modal-navigate').onclick = () => {
  if (modalRegion) {
    document.getElementById('modal-overlay').classList.remove('visible');
    const node = rootHier.descendants().find(d => d.data.region_id === modalRegion.region_id);
    if (node && node.parent) zoom(node.parent);
  }
};

async function fetchSummary(data) {
  // region id is the UUID id field in our data
  const regionUuid = data.id;
  const body = document.getElementById('modal-body');

  // Function to attach generate button event listener
  function attachGenerateButtonListener(regionId, regionData) {
    const generateBtn = document.getElementById('generate-summary-btn');
    if (!generateBtn) return;
    
    generateBtn.addEventListener('click', async function() {
      if (!confirm(`Generate a new AI summary for "${regionData.name}"?\n\nThis will create a fresh summary based on the latest research data.`)) {
        return;
      }
      
      // Disable button and show loading state
      generateBtn.disabled = true;
      generateBtn.innerHTML = '<span>⏳ Generating...</span>';
      
      try {
        const response = await fetch(`/api/regions/${regionId}/generate`, {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json'
          }
        });
        
        if (!response.ok) {
          throw new Error(`HTTP ${response.status}: ${response.statusText}`);
        }
        
        const result = await response.json();
        
        // Success
        generateBtn.innerHTML = '<span>✅ Generated</span>';
        generateBtn.style.background = 'rgba(76, 175, 80, 0.2)';
        generateBtn.style.borderColor = '#4caf50';
        
        // Show success message
        setTimeout(() => {
          body.innerHTML = `<div style="text-align:center;padding:40px;color:var(--accent2);">
            <div style="font-size:48px;margin-bottom:16px;">✅</div>
            <div style="font-size:16px;font-weight:700;margin-bottom:8px;">Summary Generation Started</div>
            <div style="font-size:12px;color:var(--text-dim);margin-bottom:16px;">A new AI summary for "${regionData.name}" is being generated.</div>
            <div style="font-size:11px;color:var(--text-dim);background:rgba(79,195,247,0.1);padding:12px;border-radius:4px;margin-bottom:20px;text-align:left;">
              <strong style="color:var(--accent);">Note:</strong> Summary generation may take a few minutes. The new summary will appear when ready.
            </div>
            <button onclick="document.getElementById('modal-overlay').click()" style="margin-top:20px;padding:10px 20px;background:var(--accent);border:none;border-radius:4px;color:#0a0c10;font-weight:700;cursor:pointer;">Close</button>
          </div>`;
        }, 1000);
        
      } catch (error) {
        console.error('Failed to generate summary:', error);
        generateBtn.innerHTML = '<span>❌ Failed</span>';
        generateBtn.style.background = 'rgba(244, 67, 54, 0.2)';
        generateBtn.style.borderColor = '#f44336';
        
        // Re-enable after 2 seconds
        setTimeout(() => {
          generateBtn.disabled = false;
          generateBtn.innerHTML = '<span>✨ Generate New Summary</span>';
          generateBtn.style.background = '';
          generateBtn.style.borderColor = '';
        }, 2000);
      }
    });
  }

  try {
    const res = await fetch(`http://capstone.ssdd.dev/orch/api/regions/${regionUuid}/summaries`);
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const json = await res.json();

    // Handle various response shapes
    const summaries = Array.isArray(json) ? json : (json.summaries || json.data || []);

    if (!summaries || summaries.length === 0) {
      body.innerHTML = `
        <div class="summary-meta">
          <div class="m"><span>Region:</span> ${data.name}</div>
          <div class="m"><span>Total Summaries:</span> 0</div>
        </div>
        <button id="generate-summary-btn" class="btn-generate-summary">
          <span>✨ Generate New Summary</span>
        </button>
        <div class="no-summary">
          No summaries available for this region yet.<br><br>
          <span style="color:var(--text-dim);font-size:11px;">Click the button above to generate the first AI summary for ${data.name} (${data.acronym})</span>
        </div>`;
      
      // Attach event listener for generate button
      attachGenerateButtonListener(regionUuid, data);
      return;
    }

    // Sort summaries by date (newest first)
    const sortedSummaries = summaries.sort((a, b) => {
      const dateA = new Date(a.created_at || a.createdAt || a.date || 0);
      const dateB = new Date(b.created_at || b.createdAt || b.date || 0);
      return dateB - dateA;
    });

    const color = data.color ? toRgb(data.color) : '#444';
    let html = `<div class="summary-meta">
      <div class="m"><span>Region:</span> ${data.name}</div>
      <div class="m"><span>Total Summaries:</span> ${summaries.length}</div>
    </div>
    <button id="generate-summary-btn" class="btn-generate-summary">
      <span>✨ Generate New Summary</span>
    </button>`;

    // Function to replace [chunk:...] with PMCID from sources
    function replaceCitationsWithPMCID(text, summary) {
      const sources = summary.sources || [];
      
      if (!Array.isArray(sources) || sources.length === 0) {
        return text;
      }
      
      // Build lookup map from chunk IDs to PMCIDs
      const idToPMCID = {};
      
      sources.forEach((source) => {
        const pmcid = source.pmc_id || source.pmcid || source.PMCID || source.pmcId;
        
        if (pmcid) {
          // Map all possible ID fields from the source to this PMCID
          // This handles cases where [chunk:X] might reference any ID field in the source
          const possibleIds = [
            source.chunk_id,
            source.chunkId,
            source.id,
            source.chunk,
            source.uid,
            source.region_id,
            source.regionId
          ].filter(id => id && typeof id === 'string'); // Filter out null/undefined values
          
          possibleIds.forEach(id => {
            idToPMCID[id.trim()] = pmcid;
          });
        }
      });
      
      if (Object.keys(idToPMCID).length === 0) {
        return text;
      }
      
      // Replace [chunk:...] with [PMCID] links
      const result = text.replace(/\[chunk:([^\]]+)\]/gi, (match, idInText) => {
        const cleanId = idInText.trim();
        const pmcid = idToPMCID[cleanId];
        
        if (pmcid) {
          // Create clickable link to PubMed Central
          return `[<a href="https://www.ncbi.nlm.nih.gov/pmc/articles/${pmcid}/" target="_blank" rel="noopener" class="summary-citation">${pmcid}</a>]`;
        }
        
        // Keep original citation if no PMCID found (data may be incomplete)
        return match;
      });
      
      return result;
    }

    // Display only the newest summary by default
    const newestSummary = sortedSummaries[0];
    let text = newestSummary.summary || newestSummary.text || newestSummary.content || newestSummary.body || newestSummary.description || JSON.stringify(newestSummary);
    const date = newestSummary.created_at || newestSummary.createdAt || newestSummary.date || '';
    const dateStr = date ? new Date(date).toLocaleDateString('en-US', {year:'numeric',month:'short',day:'numeric', hour:'2-digit', minute:'2-digit'}) : '';

    // Replace chunk citations with PMCIDs
    text = replaceCitationsWithPMCID(text, newestSummary);

    html += `<div class="summary-item">`;
    html += `<div style="font-size:10px;color:var(--accent2);letter-spacing:0.1em;text-transform:uppercase;margin-bottom:8px;font-weight:700;">Latest Summary${dateStr ? ' · ' + dateStr : ''}</div>`;
    
    // Render markdown to HTML
    const renderedText = typeof marked !== 'undefined' ? marked.parse(text) : text.replace(/\n/g, '<br>');
    html += `<div class="summary-text">${renderedText}</div>`;
    html += `</div>`;

    // If there are older summaries, add a button to view them
    if (sortedSummaries.length > 1) {
      html += `<div id="older-summaries-container" style="display:none;">`;
      
      sortedSummaries.slice(1).forEach((s, i) => {
        let olderText = s.summary || s.text || s.content || s.body || s.description || JSON.stringify(s);
        const olderDate = s.created_at || s.createdAt || s.date || '';
        const olderDateStr = olderDate ? new Date(olderDate).toLocaleDateString('en-US', {year:'numeric',month:'short',day:'numeric', hour:'2-digit', minute:'2-digit'}) : '';
        
        // Replace chunk citations with PMCIDs
        olderText = replaceCitationsWithPMCID(olderText, s);
        
        html += `<div class="summary-item" style="margin-top:24px;padding-top:20px;border-top:1px solid var(--border);">`;
        html += `<div style="font-size:10px;color:var(--text-dim);letter-spacing:0.1em;text-transform:uppercase;margin-bottom:8px;">Previous Summary ${i+1}${olderDateStr ? ' · ' + olderDateStr : ''}</div>`;
        
        const olderRenderedText = typeof marked !== 'undefined' ? marked.parse(olderText) : olderText.replace(/\n/g, '<br>');
        html += `<div class="summary-text">${olderRenderedText}</div>`;
        html += `</div>`;
      });
      
      html += `</div>`;
      
      html += `<button id="toggle-older-summaries" class="btn-toggle-summaries">
        <span id="toggle-text">View ${sortedSummaries.length - 1} Older ${sortedSummaries.length - 1 === 1 ? 'Summary' : 'Summaries'}</span>
        <span id="toggle-icon">↓</span>
      </button>`;
    }

    body.innerHTML = html;

    // Add event listener for generate button using the reusable function
    attachGenerateButtonListener(regionUuid, data);

    // Add event listener for toggle button
    const toggleBtn = document.getElementById('toggle-older-summaries');
    if (toggleBtn) {
      toggleBtn.addEventListener('click', function() {
        const container = document.getElementById('older-summaries-container');
        const icon = document.getElementById('toggle-icon');
        const text = document.getElementById('toggle-text');
        
        if (container.style.display === 'none') {
          container.style.display = 'block';
          icon.textContent = '↑';
          text.textContent = 'Hide Older Summaries';
        } else {
          container.style.display = 'none';
          icon.textContent = '↓';
          text.textContent = `View ${sortedSummaries.length - 1} Older ${sortedSummaries.length - 1 === 1 ? 'Summary' : 'Summaries'}`;
        }
      });
    }
  } catch (e) {
    body.innerHTML = `<div class="no-summary">
      Could not load summary for this region.<br>
      <span style="color:var(--text-dim);font-size:11px;margin-top:8px;display:block">Error: ${e.message}</span>
      <br><span style="color:var(--text-dim);font-size:11px;">Region UUID: ${regionUuid}</span>
    </div>`;
  }
}

// === SEARCH ===
const searchInput = document.getElementById('search-input');
const searchResults = document.getElementById('search-results');
let selectedSearchIndex = -1;

searchInput.addEventListener('input', () => {
  const q = searchInput.value.trim().toLowerCase();
  selectedSearchIndex = -1; // Reset selection on new input
  
  if (!q || q.length < 2) {
    searchResults.classList.remove('visible');
    return;
  }

  const matches = REGIONS_DATA
    .filter(r => r.name.toLowerCase().includes(q) || r.acronym.toLowerCase().includes(q))
    .slice(0, 12);

  if (matches.length === 0) {
    searchResults.classList.remove('visible');
    return;
  }

  searchResults.innerHTML = matches.map(r => {
    const cc = r.color ? toRgb(r.color) : '#444';
    return `<div class="search-result-item" data-region-id="${r.region_id}">
      <div class="swatch" style="background:${cc}"></div>
      <span class="region-name">${r.name}</span>
      <span class="region-acro">${r.acronym}</span>
    </div>`;
  }).join('');

  searchResults.querySelectorAll('.search-result-item').forEach(el => {
    el.addEventListener('click', () => {
      const id = parseInt(el.dataset.regionId);
      searchResults.classList.remove('visible');
      searchInput.value = '';
      selectedSearchIndex = -1;
      window.zoomToNodeAndPanel(id);
    });
  });

  searchResults.classList.add('visible');
});

// Keyboard navigation for search
searchInput.addEventListener('keydown', (e) => {
  const items = searchResults.querySelectorAll('.search-result-item');
  
  if (items.length === 0) return;
  
  if (e.key === 'ArrowDown') {
    e.preventDefault();
    selectedSearchIndex = Math.min(selectedSearchIndex + 1, items.length - 1);
    updateSearchSelection(items);
  } else if (e.key === 'ArrowUp') {
    e.preventDefault();
    selectedSearchIndex = Math.max(selectedSearchIndex - 1, -1);
    updateSearchSelection(items);
  } else if (e.key === 'Enter') {
    e.preventDefault();
    if (selectedSearchIndex >= 0 && selectedSearchIndex < items.length) {
      const selectedItem = items[selectedSearchIndex];
      const id = parseInt(selectedItem.dataset.regionId);
      searchResults.classList.remove('visible');
      searchInput.value = '';
      selectedSearchIndex = -1;
      window.zoomToNodeAndPanel(id);
    }
  } else if (e.key === 'Escape') {
    searchResults.classList.remove('visible');
    selectedSearchIndex = -1;
  }
});

function updateSearchSelection(items) {
  items.forEach((item, idx) => {
    if (idx === selectedSearchIndex) {
      item.classList.add('keyboard-selected');
      item.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    } else {
      item.classList.remove('keyboard-selected');
    }
  });
}

document.addEventListener('click', e => {
  if (!e.target.closest('.search-box')) {
    searchResults.classList.remove('visible');
    selectedSearchIndex = -1;
  }
});

// Initial state
updateBreadcrumb(rootHier);
updatePanel(rootHier.data);
}

// Start the application
initializeApp();
