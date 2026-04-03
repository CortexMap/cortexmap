#[derive(Debug, Clone)]
pub struct Fetcher {
    pub query: String,
    pub page_size: u64,
    pub upload_path_prefix: String,
    
    /// Timeout in seconds between processing each task in the queue
    /// This helps with rate limiting and avoiding overwhelming external APIs
    /// Default: 1 second
    pub task_timeout_secs: u64,
    
    /// Maximum number of retry attempts for failed components
    /// Default: 3 attempts
    pub max_retry_attempts: u32,
    
    /// NCBI ESearch API URL template
    /// Default: NCBI PMC ESearch endpoint
    pub esearch_url: String,
    
    /// Retry configuration
    pub retry_config: RetryConfig,
}

#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Sleep duration (in seconds) when queue is empty
    /// Default: 5 seconds
    pub empty_queue_sleep_secs: u64,
    
    /// How often (seconds) a worker refreshes its heartbeat key in Redis.
    /// Default: 15
    pub heartbeat_interval_secs: u64,

    /// TTL (seconds) for the Redis heartbeat key. Should be > 2× heartbeat_interval_secs.
    /// Default: 45
    pub heartbeat_ttl_secs: u64,

    /// Minimum idle time (ms) before XAUTOCLAIM reclaims a PEL entry.
    /// Default: 60000 (60 seconds)
    pub stale_reclaim_min_idle_ms: u64,

    /// Backoff strategy for retries
    /// Default: Constant (no backoff)
    pub backoff_strategy: BackoffStrategy,
    
    /// Different max retry attempts per component type
    /// If None, uses max_retry_attempts for all component types
    pub component_max_retries: Option<ComponentRetryConfig>,
}

#[derive(Debug, Clone)]
pub enum BackoffStrategy {
    /// Constant delay between retries (no backoff)
    Constant,
    
    /// Linear backoff: delay = base_delay * attempt
    /// Example: 1s, 2s, 3s, 4s...
    Linear {
        /// Maximum backoff time in seconds
        max_delay_secs: u64,
    },
    
    /// Exponential backoff: delay = base_delay * 2^(attempt-1)
    /// Example with 1s base: 1s, 2s, 4s, 8s, 16s...
    Exponential {
        /// Maximum backoff time in seconds
        max_delay_secs: u64,
        /// Jitter factor (0.0 to 1.0) for randomization
        /// 0.0 = no jitter, 1.0 = full jitter
        jitter: f64,
    },
    
    /// Fibonacci backoff: delay follows fibonacci sequence
    /// Example with 1s base: 1s, 1s, 2s, 3s, 5s, 8s...
    Fibonacci {
        /// Maximum backoff time in seconds
        max_delay_secs: u64,
    },
}

#[derive(Debug, Clone)]
pub struct ComponentRetryConfig {
    /// Max retries for summary fetching (metadata)
    /// Default: None (uses global max_retry_attempts)
    pub summary_max_retries: Option<u32>,
    
    /// Max retries for abstract fetching
    /// Default: None (uses global max_retry_attempts)
    pub abstract_max_retries: Option<u32>,
    
    /// Max retries for PDF fetching
    /// Default: None (uses global max_retry_attempts)
    pub pdf_max_retries: Option<u32>,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            empty_queue_sleep_secs: 5,
            heartbeat_interval_secs: 15,
            heartbeat_ttl_secs: 45,
            stale_reclaim_min_idle_ms: 60_000,
            backoff_strategy: BackoffStrategy::Constant,
            component_max_retries: None,
        }
    }
}

impl RetryConfig {
    /// Get max retries for a specific component type
    pub fn get_component_max_retries(&self, component_type: &str, global_max: u32) -> u32 {
        if let Some(ref component_config) = self.component_max_retries {
            match component_type {
                "summary" => component_config.summary_max_retries.unwrap_or(global_max),
                "abstract" => component_config.abstract_max_retries.unwrap_or(global_max),
                "pdf" => component_config.pdf_max_retries.unwrap_or(global_max),
                _ => global_max,
            }
        } else {
            global_max
        }
    }
}

impl Default for Fetcher {
    fn default() -> Self {
        Self {
            query: String::new(),
            page_size: 10,
            upload_path_prefix: String::from("papers"),
            task_timeout_secs: 1,
            max_retry_attempts: 3,
            esearch_url: String::from(
                "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi?db=pmc&term={query}&retmode=json&retmax={pageSize}"
            ),
            retry_config: RetryConfig::default(),
        }
    }
}
