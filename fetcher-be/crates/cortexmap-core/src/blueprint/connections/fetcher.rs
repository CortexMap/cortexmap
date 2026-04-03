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

    /// Multiplier for timeout on stale task detection
    /// Tasks in "in_progress" for more than (task_timeout_secs * stale_task_multiplier) are considered stale
    /// Default: 10 (e.g., 10x timeout means 10 seconds for 1 second timeout)
    pub stale_task_multiplier: u64,

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
            stale_task_multiplier: 10,
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
                "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi?db=pmc&term={query}&retmode=json&retmax={pageSize}",
            ),
            retry_config: RetryConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retry_config_defaults_are_safe() {
        let config = RetryConfig::default();

        assert_eq!(config.empty_queue_sleep_secs, 5);
        assert_eq!(config.stale_task_multiplier, 10);
        assert!(matches!(config.backoff_strategy, BackoffStrategy::Constant));
        assert!(config.component_max_retries.is_none());
    }

    #[test]
    fn test_fetcher_defaults_match_expected_runtime_values() {
        let fetcher = Fetcher::default();

        assert_eq!(fetcher.query, "");
        assert_eq!(fetcher.page_size, 10);
        assert_eq!(fetcher.upload_path_prefix, "papers");
        assert_eq!(fetcher.task_timeout_secs, 1);
        assert_eq!(fetcher.max_retry_attempts, 3);
        assert!(fetcher.esearch_url.contains("{query}"));
        assert!(fetcher.esearch_url.contains("{pageSize}"));
        assert!(matches!(
            fetcher.retry_config.backoff_strategy,
            BackoffStrategy::Constant
        ));
    }

    #[test]
    fn test_component_retry_overrides_take_precedence() {
        let config = RetryConfig {
            component_max_retries: Some(ComponentRetryConfig {
                summary_max_retries: Some(5),
                abstract_max_retries: Some(4),
                pdf_max_retries: Some(2),
            }),
            ..RetryConfig::default()
        };

        assert_eq!(config.get_component_max_retries("summary", 3), 5);
        assert_eq!(config.get_component_max_retries("abstract", 3), 4);
        assert_eq!(config.get_component_max_retries("pdf", 3), 2);
    }

    #[test]
    fn test_component_retry_overrides_fall_back_to_global_when_missing() {
        let config = RetryConfig {
            component_max_retries: Some(ComponentRetryConfig {
                summary_max_retries: None,
                abstract_max_retries: Some(6),
                pdf_max_retries: None,
            }),
            ..RetryConfig::default()
        };

        assert_eq!(config.get_component_max_retries("summary", 3), 3);
        assert_eq!(config.get_component_max_retries("abstract", 3), 6);
        assert_eq!(config.get_component_max_retries("pdf", 3), 3);
        assert_eq!(config.get_component_max_retries("unknown", 3), 3);
        assert_eq!(
            RetryConfig::default().get_component_max_retries("summary", 7),
            7
        );
    }
}
