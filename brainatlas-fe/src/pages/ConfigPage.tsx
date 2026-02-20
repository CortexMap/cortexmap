import { useState, useEffect } from 'react';
import { api } from '../api';
import { ConfigEntry } from '../types';
import './ConfigPage.css';

interface GroupedConfig {
  [category: string]: ConfigEntry[];
}

export default function ConfigPage() {
  const [config, setConfig] = useState<ConfigEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [success, setSuccess] = useState('');
  const [searchTerm, setSearchTerm] = useState('');
  const [tempValues, setTempValues] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);
  const [showConfirmModal, setShowConfirmModal] = useState(false);

  useEffect(() => {
    loadConfig();
  }, []);

  const loadConfig = async () => {
    try {
      setError('');
      const entries = await api.getConfig();
      setConfig(entries);
      setTempValues({}); // Clear temp values on load
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setLoading(false);
    }
  };

  const hasChanges = () => {
    return Object.keys(tempValues).some(key => {
      const entry = config.find(e => e.key === key);
      return entry && tempValues[key] !== entry.value;
    });
  };

  const getChanges = () => {
    return Object.keys(tempValues)
      .filter(key => {
        const entry = config.find(e => e.key === key);
        return entry && tempValues[key] !== entry.value;
      })
      .map(key => {
        const entry = config.find(e => e.key === key)!;
        return {
          key,
          oldValue: entry.value,
          newValue: tempValues[key],
          description: entry.description
        };
      });
  };

  const handleSaveChanges = async () => {
    const changes = getChanges();

    if (changes.length === 0) {
      setSuccess('No changes to save');
      setTimeout(() => setSuccess(''), 3000);
      return;
    }

    // Show confirmation modal
    setShowConfirmModal(true);
  };

  const handleConfirmSave = async () => {
    const changes = getChanges();
    
    setShowConfirmModal(false);
    setSaving(true);
    setError('');
    setSuccess('');

    try {
      const updates = changes.map(change => ({
        key: change.key,
        value: change.newValue
      }));

      const updated = await api.updateConfig(updates);
      
      setConfig(prev => prev.map(e => 
        updated.find(u => u.key === e.key) || e
      ));
      
      setTempValues({});
      setSuccess(`Successfully updated ${changes.length} configuration${changes.length > 1 ? 's' : ''}`);
      setTimeout(() => setSuccess(''), 5000);
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setSaving(false);
    }
  };

  const handleCancelConfirm = () => {
    setShowConfirmModal(false);
  };

  const handleDiscardChanges = () => {
    setTempValues({});
    setSuccess('Changes discarded');
    setTimeout(() => setSuccess(''), 3000);
  };

  const categorizeConfig = (entries: ConfigEntry[]): GroupedConfig => {
    const grouped: GroupedConfig = {
      'Database': [],
      'Fetcher Service': [],
      'LLM & AI': [],
      'Processing': [],
      'Other': []
    };

    entries.forEach(entry => {
      const key = entry.key.toLowerCase();
      if (key.includes('db') || key.includes('database') || key.includes('postgres')) {
        grouped['Database'].push(entry);
      } else if (key.includes('fetch') || key.includes('worker') || key.includes('queue')) {
        grouped['Fetcher Service'].push(entry);
      } else if (key.includes('llm') || key.includes('openai') || key.includes('model') || key.includes('ai')) {
        grouped['LLM & AI'].push(entry);
      } else if (key.includes('batch') || key.includes('chunk') || key.includes('task') || key.includes('timeout')) {
        grouped['Processing'].push(entry);
      } else {
        grouped['Other'].push(entry);
      }
    });

    // Remove empty categories
    Object.keys(grouped).forEach(key => {
      if (grouped[key].length === 0) delete grouped[key];
    });

    return grouped;
  };

  const filteredConfig = config.filter(entry =>
    entry.key.toLowerCase().includes(searchTerm.toLowerCase()) ||
    entry.description.toLowerCase().includes(searchTerm.toLowerCase()) ||
    entry.value.toLowerCase().includes(searchTerm.toLowerCase())
  );

  const groupedConfig = categorizeConfig(filteredConfig);

  if (loading) {
    return (
      <div className="config-page">
        <div className="loading-state">
          <div className="spinner"></div>
          <p>Loading configuration...</p>
        </div>
      </div>
    );
  }

  return (
    <div className="config-page">
      <header className="config-header">
        <div className="header-content">
          <h1>Configuration Management</h1>
          <p className="subtitle">Manage system settings and environment variables</p>
        </div>
        
        <div className="search-bar">
          <input
            type="text"
            placeholder="Search configurations..."
            value={searchTerm}
            onChange={e => setSearchTerm(e.target.value)}
          />
        </div>

        {error && (
          <div className="alert alert-error">
            <strong>Error:</strong> {error}
          </div>
        )}
        
        {success && (
          <div className="alert alert-success">
            {success}
          </div>
        )}
      </header>

      <div className="config-content">
        {Object.entries(groupedConfig).map(([category, entries]) => (
          <section key={category} className="config-category">
            <h2 className="category-title">{category}</h2>
            <div className="config-grid">
              {entries.map(entry => {
                const currentValue = tempValues[entry.key] ?? entry.value;
                const isModified = tempValues[entry.key] !== undefined && tempValues[entry.key] !== entry.value;
                
                return (
                  <div key={entry.key} className={`config-card ${isModified ? 'modified' : ''}`}>
                    <div className="card-header">
                      <div className="config-key">
                        {entry.key}
                        {isModified && <span className="modified-indicator"> (modified)</span>}
                      </div>
                      {entry.updated_at && (
                        <div className="last-updated">
                          Last updated: {new Date(entry.updated_at).toLocaleDateString()} at {new Date(entry.updated_at).toLocaleTimeString()}
                        </div>
                      )}
                    </div>
                    
                    <div className="config-description">{entry.description}</div>
                    
                    <div className="config-input-group">
                      <input
                        type="text"
                        className="config-input"
                        value={currentValue}
                        onChange={e => setTempValues(prev => ({ ...prev, [entry.key]: e.target.value }))}
                        disabled={saving}
                      />
                    </div>
                  </div>
                );
              })}
            </div>
          </section>
        ))}
        
        {filteredConfig.length === 0 && (
          <div className="empty-state">
            <p>No configurations match your search</p>
          </div>
        )}
      </div>

      {hasChanges() && (
        <div className="config-actions">
          <button
            className="discard-btn"
            onClick={handleDiscardChanges}
            disabled={saving}
          >
            Discard Changes
          </button>
          <button
            className="save-btn"
            onClick={handleSaveChanges}
            disabled={saving}
          >
            {saving ? 'Saving...' : `Save ${Object.keys(tempValues).filter(k => tempValues[k] !== config.find(e => e.key === k)?.value).length} Change${Object.keys(tempValues).filter(k => tempValues[k] !== config.find(e => e.key === k)?.value).length > 1 ? 's' : ''}`}
          </button>
        </div>
      )}

      {/* Confirmation Modal */}
      {showConfirmModal && (
        <div className="modal-overlay" onClick={handleCancelConfirm}>
          <div className="modal-content" onClick={e => e.stopPropagation()}>
            <div className="modal-header">
              <h2>Confirm Configuration Changes</h2>
              <button className="modal-close" onClick={handleCancelConfirm}>✕</button>
            </div>
            
            <div className="modal-body">
              <p className="modal-warning">
                Are you sure you want to update the following configuration{getChanges().length > 1 ? 's' : ''}?
              </p>
              
              <div className="changes-list">
                {getChanges().map(change => (
                  <div key={change.key} className="change-item">
                    <div className="change-key">{change.key}</div>
                    <div className="change-description">{change.description}</div>
                    <div className="change-diff">
                      <div className="diff-row old-value">
                        <span className="diff-label">Current:</span>
                        <code className="diff-value">{change.oldValue}</code>
                      </div>
                      <div className="diff-arrow">→</div>
                      <div className="diff-row new-value">
                        <span className="diff-label">New:</span>
                        <code className="diff-value">{change.newValue}</code>
                      </div>
                    </div>
                  </div>
                ))}
              </div>
            </div>
            
            <div className="modal-footer">
              <button className="modal-cancel-btn" onClick={handleCancelConfirm}>
                Cancel
              </button>
              <button className="modal-confirm-btn" onClick={handleConfirmSave}>
                Relax. I've got this.
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
