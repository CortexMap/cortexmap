import { useState, useEffect } from 'react';
import axios from 'axios';
import { BarChart, Bar, XAxis, YAxis, CartesianGrid, Tooltip, ResponsiveContainer, Cell } from 'recharts';
import { Activity, TrendingUp } from 'lucide-react';
import { API_BASE_URL, logger } from '../config';
import './PipelineStats.css';

const STATUS_COLORS = {
  done: '#10b981',
  processing: '#8b5cf6',
  llm_queued: '#a855f7',
  fetching: '#3b82f6',
  fetch_queued: '#60a5fa',
  not_started: '#64748b',
  fetch_failed: '#ef4444',
  invalidated: '#f59e0b',
};

function PipelineStats() {
  const [stats, setStats] = useState(null);
  const [loading, setLoading] = useState(true);
  const [collapsed, setCollapsed] = useState(false);

  useEffect(() => {
    logger.info('PipelineStats component mounted');
    fetchStats();
    const interval = setInterval(() => {
      logger.debug('Auto-refreshing pipeline stats');
      fetchStats();
    }, 10000); // Refresh every 10 seconds
    return () => {
      logger.debug('Clearing pipeline stats interval');
      clearInterval(interval);
    };
  }, []);

  const fetchStats = async () => {
    const url = `${API_BASE_URL}/pipeline/stats`;
    logger.api('GET', url);
    
    try {
      const response = await axios.get(url);
      logger.apiSuccess('GET', url, response.data);
      setStats(response.data);
      setLoading(false);
    } catch (err) {
      logger.apiError('GET', url, err);
      logger.warn('Pipeline stats unavailable - component will continue without stats');
      setLoading(false);
    }
  };

  if (loading) {
    return (
      <div className="pipeline-stats loading">
        <Activity size={24} className="spinning" />
        <span>Loading pipeline statistics...</span>
      </div>
    );
  }

  if (!stats) {
    return null;
  }

  const chartData = [
    { name: 'Done', value: stats.done, color: STATUS_COLORS.done },
    { name: 'Processing', value: stats.processing, color: STATUS_COLORS.processing },
    { name: 'LLM Queued', value: stats.llm_queued, color: STATUS_COLORS.llm_queued },
    { name: 'Fetching', value: stats.fetching, color: STATUS_COLORS.fetching },
    { name: 'Fetch Queued', value: stats.fetch_queued, color: STATUS_COLORS.fetch_queued },
    { name: 'Not Started', value: stats.not_started, color: STATUS_COLORS.not_started },
    { name: 'Failed', value: stats.fetch_failed, color: STATUS_COLORS.fetch_failed },
    { name: 'Invalidated', value: stats.invalidated, color: STATUS_COLORS.invalidated },
  ].filter(item => item.value > 0);

  const completionRate = stats.total_regions > 0 
    ? ((stats.done / stats.total_regions) * 100).toFixed(1) 
    : 0;

  return (
    <div className={`pipeline-stats-container ${collapsed ? 'collapsed' : ''}`}>
      <div className="stats-header" onClick={() => setCollapsed(!collapsed)}>
        <div className="stats-title">
          <Activity size={24} />
          <h2>Pipeline Statistics</h2>
        </div>
        <div className="stats-summary">
          <div className="summary-item">
            <TrendingUp size={18} />
            <span>{completionRate}% Complete</span>
          </div>
          <span className="total-regions">{stats.total_regions} Total Regions</span>
          <button className="collapse-btn">{collapsed ? '▼' : '▲'}</button>
        </div>
      </div>

      {!collapsed && (
        <div className="stats-content">
          <div className="stats-grid">
            <StatCard label="Completed" value={stats.done} color={STATUS_COLORS.done} />
            <StatCard label="Processing" value={stats.processing} color={STATUS_COLORS.processing} />
            <StatCard label="LLM Queued" value={stats.llm_queued} color={STATUS_COLORS.llm_queued} />
            <StatCard label="Fetching" value={stats.fetching} color={STATUS_COLORS.fetching} />
            <StatCard label="Fetch Queued" value={stats.fetch_queued} color={STATUS_COLORS.fetch_queued} />
            <StatCard label="Not Started" value={stats.not_started} color={STATUS_COLORS.not_started} />
            <StatCard label="Failed" value={stats.fetch_failed} color={STATUS_COLORS.fetch_failed} />
            <StatCard label="Invalidated" value={stats.invalidated} color={STATUS_COLORS.invalidated} />
          </div>

          {chartData.length > 0 && (
            <div className="chart-container">
              <h3>Status Distribution</h3>
              <ResponsiveContainer width="100%" height={250}>
                <BarChart data={chartData}>
                  <CartesianGrid strokeDasharray="3 3" stroke="#475569" />
                  <XAxis 
                    dataKey="name" 
                    stroke="#cbd5e1" 
                    tick={{ fill: '#cbd5e1', fontSize: 12 }}
                    angle={-45}
                    textAnchor="end"
                    height={80}
                  />
                  <YAxis stroke="#cbd5e1" tick={{ fill: '#cbd5e1' }} />
                  <Tooltip 
                    contentStyle={{ 
                      backgroundColor: '#1e293b', 
                      border: '2px solid #475569',
                      borderRadius: '8px',
                      color: '#f1f5f9'
                    }}
                  />
                  <Bar dataKey="value" radius={[8, 8, 0, 0]}>
                    {chartData.map((entry, index) => (
                      <Cell key={`cell-${index}`} fill={entry.color} />
                    ))}
                  </Bar>
                </BarChart>
              </ResponsiveContainer>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function StatCard({ label, value, color }) {
  return (
    <div className="stat-card" style={{ borderLeftColor: color }}>
      <div className="stat-label">{label}</div>
      <div className="stat-value" style={{ color }}>{value.toLocaleString()}</div>
    </div>
  );
}

export default PipelineStats;
