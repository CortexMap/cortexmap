import { useState, useEffect, useRef } from 'react';
import { api } from '../api';
import { chatStorage } from '../utils/storage';
import { useWorkers } from '../hooks/useWorkers';
import {
  ChatMessage,
  RegionMapping,
  RegionSummary,
  BatchStatusResult,
  ChunkSourceResponse,
  ConfigEntry
} from '../types';
import './ChatPage.css';

export default function ChatPage() {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState('');
  const [sending, setSending] = useState(false);
  const [regions, setRegions] = useState<RegionMapping[]>([]);
  const [activeBatches, setActiveBatches] = useState<Map<number, string>>(new Map());
  const [expandedSources, setExpandedSources] = useState<Set<string>>(new Set());
  const [chunkDetails, setChunkDetails] = useState<Map<string, ChunkSourceResponse>>(new Map());
  const [loadingChunks, setLoadingChunks] = useState<Set<string>>(new Set());
  const [workerCountToAllocate, setWorkerCountToAllocate] = useState(2);
  const [showConfig, setShowConfig] = useState(false);
  const [showWorkers, setShowWorkers] = useState(false);
  const [config, setConfig] = useState<ConfigEntry[]>([]);
  const [loadingConfig, setLoadingConfig] = useState(false);
  const [configError, setConfigError] = useState('');
  
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const pollIntervalRef = useRef<NodeJS.Timeout | null>(null);

  // Use the workers hook
  const { workers, allocate, stop, allocating, stopping, error: workerError } = useWorkers();

  // Load chat history on mount
  useEffect(() => {
    const savedMessages = chatStorage.load();
    setMessages(savedMessages);
    
    // Resume batch polling for any in-progress messages
    const inProgressBatches = new Map<number, string>();
    savedMessages.forEach((msg, index) => {
      if (msg.batchId && msg.role === 'assistant' && !msg.content.includes('Summaries:')) {
        inProgressBatches.set(index, msg.batchId);
      }
    });
    setActiveBatches(inProgressBatches);
    
    loadRegions();
    
    return () => {
      if (pollIntervalRef.current) clearInterval(pollIntervalRef.current);
    };
  }, []);

  // Save messages whenever they change
  useEffect(() => {
    chatStorage.save(messages);
  }, [messages]);

  // Auto-scroll to bottom on new messages
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages]);

  // Poll active batches
  useEffect(() => {
    if (activeBatches.size > 0 && !pollIntervalRef.current) {
      pollIntervalRef.current = setInterval(checkActiveBatches, 3000);
    } else if (activeBatches.size === 0 && pollIntervalRef.current) {
      clearInterval(pollIntervalRef.current);
      pollIntervalRef.current = null;
    }
  }, [activeBatches.size]);

  const loadRegions = async () => {
    try {
      const data = await api.listBrainRegions();
      setRegions(data);
    } catch (err) {
      console.error('Failed to load regions:', err);
    }
  };

  const checkActiveBatches = async () => {
    const updates = new Map<number, string>();
    const completedBatches: number[] = [];
    
    for (const [msgIndex, batchId] of activeBatches.entries()) {
      try {
        const status: BatchStatusResult = await api.getBatchStatus(batchId);
        
        // Update the message with progress
        setMessages(prev => prev.map((msg, idx) => {
          if (idx === msgIndex) {
            const progressText = `${status.message}\n\nProgress: ${status.completed_tasks || 0}/${status.expected_tasks} tasks`;
            return { ...msg, content: progressText };
          }
          return msg;
        }));
        
        // Check if batch is complete
        if (status.status === 'Done') {
          completedBatches.push(msgIndex);
          // Fetch the final summaries
          const regionId = messages[msgIndex].regionId;
          if (regionId) {
            const summariesResp = await api.getSummaries(regionId);
            setMessages(prev => prev.map((msg, idx) => {
              if (idx === msgIndex) {
                return {
                  ...msg,
                  content: formatSummaries(summariesResp.summaries),
                  batchId: undefined // Clear batch ID
                };
              }
              return msg;
            }));
          }
        } else if (status.status === 'FetchFailed' || status.error) {
          completedBatches.push(msgIndex);
          setMessages(prev => prev.map((msg, idx) => {
            if (idx === msgIndex) {
              return {
                ...msg,
                content: `Error: ${status.error || 'Batch failed'}`,
                batchId: undefined
              };
            }
            return msg;
          }));
        } else {
          updates.set(msgIndex, batchId);
        }
      } catch (err) {
        console.error('Polling error for batch:', batchId, err);
        updates.set(msgIndex, batchId);
      }
    }
    
    // Remove completed batches
    if (completedBatches.length > 0) {
      setActiveBatches(prev => {
        const next = new Map(prev);
        completedBatches.forEach(idx => next.delete(idx));
        return next;
      });
    } else {
      setActiveBatches(updates);
    }
  };

  const formatSummaries = (summaries: RegionSummary[]): string => {
    if (summaries.length === 0) return 'No summaries available yet.';
    
    return summaries.map((summary, i) => 
      `**Summary ${i + 1}**\n${summary.summary}\n\n*${summary.sources.length} sources*`
    ).join('\n\n---\n\n');
  };

  const findRegionByName = (name: string): { region: RegionMapping; matchType: string } | undefined => {
    const normalized = name.toLowerCase().trim();
    
    // Debug: Log search attempt and sample regions
    console.log(`Searching for region: "${normalized}" in ${regions.length} regions`);
    if (regions.length > 0) {
      console.log('Sample regions:', regions.slice(0, 5).map(r => ({ name: r.name, acronym: r.acronym })));
      // Log any regions that contain "hipp"
      const hippRegions = regions.filter(r => r.name.toLowerCase().includes('hipp'));
      console.log(`Regions containing "hipp": ${hippRegions.length}`, hippRegions.slice(0, 3).map(r => r.name));
    }
    
    // Try exact acronym match first
    let match = regions.find(r => r.acronym?.toLowerCase() === normalized);
    if (match) {
      console.log(`Found exact acronym match: ${match.name} (${match.acronym})`);
      return { region: match, matchType: 'exact acronym' };
    }
    
    // Try exact name match
    match = regions.find(r => r.name.toLowerCase() === normalized);
    if (match) {
      console.log(`Found exact name match: ${match.name}`);
      return { region: match, matchType: 'exact name' };
    }
    
    // Try partial name match (contains) - for words like "hippocampus"
    match = regions.find(r => r.name.toLowerCase().includes(normalized));
    if (match) {
      console.log(`Found partial match: ${match.name} contains "${normalized}"`);
      return { region: match, matchType: 'partial match' };
    }
    
    // Try reverse partial match - if the input is contained in the region name
    match = regions.find(r => normalized.length >= 3 && r.name.toLowerCase().includes(normalized));
    if (match) {
      console.log(`Found fuzzy match: ${match.name} contains "${normalized}"`);
      return { region: match, matchType: 'fuzzy match' };
    }
    
    // Try acronym partial match
    match = regions.find(r => r.acronym && r.acronym.toLowerCase().includes(normalized));
    if (match) {
      console.log(`Found acronym partial match: ${match.acronym}`);
      return { region: match, matchType: 'acronym partial match' };
    }
    
    console.log(`No match found for "${normalized}"`);
    return undefined;
  };

  const handleSend = async () => {
    if (!input.trim() || sending) return;

    const userMsg: ChatMessage = {
      role: 'user',
      content: input,
      timestamp: new Date().toISOString()
    };
    setMessages(prev => [...prev, userMsg]);
    setInput('');
    setSending(true);

    try {
      const result = findRegionByName(input);
      
      if (!result) {
        const suggestions = regions.length > 0 
          ? `Try: ${regions.slice(0, 3).map(r => r.acronym || r.name).join(', ')}`
          : 'Loading regions...';
        
        setMessages(prev => [...prev, {
          role: 'assistant',
          content: `Region "${input}" not found. ${suggestions}`,
          timestamp: new Date().toISOString()
        }]);
        setSending(false);
        return;
      }

      const { region, matchType } = result;

      // Show region resolution confirmation
      const confirmMsg = `**Found region: ${region.name}**\n` +
        `Acronym: ${region.acronym || 'N/A'}\n` +
        `Match type: ${matchType}\n` +
        `Region ID: ${region.id}\n\n` +
        `Checking for existing summaries...`;

      setMessages(prev => [...prev, {
        role: 'assistant',
        content: confirmMsg,
        timestamp: new Date().toISOString(),
        regionId: region.id
      }]);

      // First, check if summaries already exist
      const summariesResp = await api.getSummaries(region.id);
      
      if (summariesResp.summaries && summariesResp.summaries.length > 0) {
        // Summaries exist, display them immediately
        setMessages(prev => {
          const newMsgs = [...prev];
          newMsgs[newMsgs.length - 1] = {
            ...newMsgs[newMsgs.length - 1],
            content: `**${region.name}** (${region.acronym})\nRegion ID: ${region.id}\n\n${formatSummaries(summariesResp.summaries)}`
          };
          return newMsgs;
        });
      } else {
        // No summaries, trigger generation
        const generateResp = await api.generateSummary(region.id);
        const msgIndex = messages.length + 1; // +1 because we just added user message
        
        setMessages(prev => {
          const newMsgs = [...prev];
          newMsgs[newMsgs.length - 1] = {
            ...newMsgs[newMsgs.length - 1],
            content: `**${region.name}** (${region.acronym})\n` +
              `Region ID: ${region.id}\n\n` +
              `**Generating summary...**\n` +
              `Batch ID: ${generateResp.batch_id}\n` +
              `Queries: ${generateResp.query_count}\n` +
              `Tasks: ${generateResp.task_count}\n\n` +
              `Processing...`,
            batchId: generateResp.batch_id
          };
          return newMsgs;
        });
        
        // Start polling this batch
        setActiveBatches(prev => new Map(prev).set(msgIndex, generateResp.batch_id));
      }
    } catch (err) {
      setMessages(prev => [...prev, {
        role: 'assistant',
        content: 'Error: ' + (err as Error).message,
        timestamp: new Date().toISOString()
      }]);
    } finally {
      setSending(false);
    }
  };

  const toggleSource = (chunkId: string) => {
    setExpandedSources(prev => {
      const next = new Set(prev);
      if (next.has(chunkId)) {
        next.delete(chunkId);
      } else {
        next.add(chunkId);
        // Fetch chunk details if not already loaded
        if (!chunkDetails.has(chunkId) && !loadingChunks.has(chunkId)) {
          loadChunkDetails(chunkId);
        }
      }
      return next;
    });
  };

  const loadChunkDetails = async (chunkId: string) => {
    setLoadingChunks(prev => new Set(prev).add(chunkId));
    try {
      const details = await api.getChunkSource(chunkId);
      setChunkDetails(prev => new Map(prev).set(chunkId, details));
    } catch (err) {
      console.error('Failed to load chunk details:', err);
    } finally {
      setLoadingChunks(prev => {
        const next = new Set(prev);
        next.delete(chunkId);
        return next;
      });
    }
  };

  const clearHistory = () => {
    setMessages([]);
    setActiveBatches(new Map());
    chatStorage.clear();
  };

  const loadConfig = async () => {
    setLoadingConfig(true);
    setConfigError('');
    try {
      const entries = await api.getConfig();
      setConfig(entries);
    } catch (err) {
      setConfigError((err as Error).message);
    } finally {
      setLoadingConfig(false);
    }
  };

  const handleConfigUpdate = async (key: string, value: string) => {
    const entry = config.find(e => e.key === key);
    if (!entry) return;
    
    try {
      setConfigError('');
      const updated = await api.updateConfig([{ ...entry, value }]);
      setConfig(prev => prev.map(e => 
        updated.find(u => u.key === e.key) || e
      ));
    } catch (err) {
      setConfigError((err as Error).message);
    }
  };

  const toggleConfig = () => {
    if (!showConfig && config.length === 0) {
      loadConfig();
    }
    setShowConfig(!showConfig);
    if (!showConfig) setShowWorkers(false); // Close workers when opening config
  };

  const toggleWorkers = () => {
    setShowWorkers(!showWorkers);
    if (!showWorkers) setShowConfig(false); // Close config when opening workers
  };

  const renderMessage = (msg: ChatMessage, index: number) => {
    const isProcessing = msg.batchId && activeBatches.has(index);
    
    return (
      <div key={index} className={`message ${msg.role}`}>
        <div className="message-header">
          <strong>{msg.role === 'user' ? 'You' : 'Assistant'}</strong>
          <span className="timestamp">{new Date(msg.timestamp).toLocaleString()}</span>
        </div>
        <div className={`message-content ${isProcessing ? 'processing' : ''}`}>
          {msg.content.split('\n').map((line, i) => {
            // Simple markdown rendering
            if (line.startsWith('**') && line.endsWith('**')) {
              return <div key={i} className="message-bold">{line.slice(2, -2)}</div>;
            }
            if (line.startsWith('*') && line.endsWith('*')) {
              return <div key={i} className="message-italic">{line.slice(1, -1)}</div>;
            }
            if (line === '---') {
              return <hr key={i} />;
            }
            return <div key={i}>{line}</div>;
          })}
        </div>
      </div>
    );
  };

  return (
    <div className="chat-page dark-theme">
      <div className="chat-header">
        <div className="header-left">
          <h2>CortexMap</h2>
          <span className="subtitle">Brain Atlas Intelligence</span>
        </div>
        <div className="header-actions">
          <button 
            onClick={toggleWorkers} 
            className={`header-btn ${showWorkers ? 'active' : ''}`}
            title="Worker Management"
          >
            Workers
            {workers.length > 0 && (
              <span className="badge">{workers.length}</span>
            )}
          </button>
          
          <button 
            onClick={toggleConfig} 
            className={`header-btn ${showConfig ? 'active' : ''}`}
            title="Settings"
          >
            Settings
          </button>
          
          <button onClick={clearHistory} className="header-btn" title="Clear chat">
            Clear
          </button>
          
          <div className="region-count">
            {regions.length} regions
          </div>
        </div>
      </div>
      
      {workerError && (
        <div className="worker-error">{workerError}</div>
      )}
      
      <div className="main-content">
        {showWorkers && (
          <div className="workers-sidebar">
            <div className="sidebar-header">
              <h3>Worker Management</h3>
              <button onClick={() => setShowWorkers(false)} className="close-sidebar">
                ✕
              </button>
            </div>
            {workerError && (
              <div className="sidebar-error">{workerError}</div>
            )}
            <div className="sidebar-content">
              <div className="add-worker-section">
                <button
                  onClick={() => allocate(1)}
                  disabled={allocating}
                  className="add-worker-button"
                >
                  {allocating ? 'Adding Worker...' : '+ Add Worker'}
                </button>
              </div>
              
              {workers.length === 0 ? (
                <div className="empty-workers">
                  <div className="empty-icon">W</div>
                  <p>No active workers</p>
                  <span>Click "+ Add Worker" to start processing tasks</span>
                </div>
              ) : (
                <div className="worker-grid">
                  {workers.map(w => (
                    <div key={w.worker_id} className="worker-card">
                      <div className="worker-card-header">
                        <span className="worker-name" title={w.worker_id}>
                          Worker {w.worker_id.substring(0, 8)}
                        </span>
                        <span className={`status-badge ${w.status}`}>
                          {w.status}
                        </span>
                      </div>
                      <div className="worker-card-stats">
                        <div className="stat">
                          <span className="stat-label">Tasks</span>
                          <span className="stat-value">{w.tasks_processed || 0}</span>
                        </div>
                        <div className="stat">
                          <span className="stat-label">Failed</span>
                          <span className="stat-value error">{w.tasks_failed || 0}</span>
                        </div>
                        {w.uptime_seconds && (
                          <div className="stat">
                            <span className="stat-label">Uptime</span>
                            <span className="stat-value">{Math.floor(w.uptime_seconds / 60)}m</span>
                          </div>
                        )}
                      </div>
                      {w.current_task && (
                        <div className="worker-current-task">
                          Processing: PMC{w.current_task}
                        </div>
                      )}
                      <button
                        onClick={() => stop(w.worker_id)}
                        disabled={stopping.has(w.worker_id)}
                        className="delete-worker-btn"
                      >
                        {stopping.has(w.worker_id) ? 'Stopping...' : 'Delete Worker'}
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </div>
        )}
        
        {showConfig && (
          <div className="config-sidebar">
            <div className="config-header">
              <h3>Configuration</h3>
              <button onClick={() => setShowConfig(false)} className="close-sidebar">
                ✕
              </button>
            </div>
            {configError && (
              <div className="config-error">{configError}</div>
            )}
            {loadingConfig ? (
              <div className="config-loading">Loading...</div>
            ) : (
              <div className="config-list">
                {config.map(entry => (
                  <div key={entry.key} className="config-item">
                    <label>
                      <span className="config-key">{entry.key}</span>
                      <span className="config-desc">{entry.description}</span>
                    </label>
                    <input
                      type="text"
                      defaultValue={entry.value}
                      onBlur={e => handleConfigUpdate(entry.key, e.target.value)}
                      className="config-input"
                    />
                    {entry.updated_at && (
                      <span className="config-updated">
                        Updated: {new Date(entry.updated_at).toLocaleString()}
                      </span>
                    )}
                  </div>
                ))}
              </div>
            )}
          </div>
        )}
        
        <div className="messages-container">
          <div className="messages">
            {messages.length === 0 && (
              <div className="empty-state">
                <div className="empty-icon">Brain</div>
                <h3>Welcome to CortexMap</h3>
                <p>Ask about any brain region to get AI-powered summaries</p>
                <div className="example-queries">
                  <span className="example">Try: "hippocampus"</span>
                  <span className="example">Try: "CA1"</span>
                  <span className="example">Try: "visual cortex"</span>
                </div>
              </div>
            )}
            {messages.map((msg, i) => renderMessage(msg, i))}
            <div ref={messagesEndRef} />
          </div>

          <div className="input-area">
            <input
              value={input}
              onChange={e => setInput(e.target.value)}
              onKeyDown={e => e.key === 'Enter' && !e.shiftKey && handleSend()}
              placeholder="Ask about a brain region... (e.g., hippocampus, CA1, visual cortex)"
              disabled={sending}
              className="chat-input"
            />
            <button onClick={handleSend} disabled={sending || !input.trim()} className="send-btn">
              {sending ? 'Sending...' : 'Send'}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
