use anyhow::Result;
use clap::{Parser, ValueEnum};
use cortexmap_core::blueprint::connections::{
    BackoffStrategy, ComponentRetryConfig, Connections, Database, Fetcher, Postgresql, RetryConfig,
    S3Info,
};
use cortexmap_core::blueprint::Blueprint;
use cortexmap_infra::TaskQueueInfra;
use futures::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::sync::Arc;
use std_infra::StdInfraContextBuilder;
use tracing::Level;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Mode {
    /// Synchronous mode: fetch and upload immediately (legacy behavior)
    Sync,
    /// Enqueue mode: add tasks to queue and exit
    Enqueue,
    /// Worker mode: run worker loop to process queue
    Worker,
    /// Status mode: show queue statistics
    Status,
}

#[derive(Parser, Debug)]
#[command(name = "cortexmap-cli")]
#[command(about = "Fetch and store academic papers from Europe PMC", long_about = None)]
struct Args {
    /// Execution mode
    #[arg(short, long, value_enum, default_value = "sync")]
    mode: Mode,

    /// Search query for Europe PMC
    #[arg(short, long)]
    query: String,

    /// Number of results to fetch
    #[arg(short = 'n', long, default_value = "10")]
    page_size: u64,

    /// S3 upload path prefix
    #[arg(short, long, default_value = "papers")]
    upload_prefix: String,

    /// Database connection URL
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    /// S3 endpoint URL
    #[arg(long, env = "S3_ENDPOINT")]
    s3_endpoint: String,

    /// S3 access key
    #[arg(long, env = "S3_ACCESS_KEY")]
    s3_access_key: String,

    /// S3 secret key
    #[arg(long, env = "S3_SECRET_KEY")]
    s3_secret_key: String,

    /// S3 bucket name
    #[arg(long, env = "S3_BUCKET")]
    s3_bucket: String,

    /// Timeout in seconds between processing each task in the queue
    #[arg(long, default_value = "1")]
    task_timeout_secs: u64,

    /// Maximum number of retry attempts for failed components
    #[arg(long, default_value = "3")]
    max_retry_attempts: u32,

    /// Sleep duration in seconds when queue is empty
    #[arg(long, default_value = "5")]
    empty_queue_sleep_secs: u64,

    /// Backoff strategy: constant, linear, exponential, fibonacci
    #[arg(long, default_value = "constant")]
    backoff_strategy: String,

    /// Maximum backoff delay in seconds (for non-constant strategies)
    #[arg(long, default_value = "300")]
    max_backoff_delay_secs: u64,

    /// Jitter factor for exponential backoff (0.0 to 1.0)
    #[arg(long, default_value = "0.1")]
    backoff_jitter: f64,

    /// Multiplier for stale task timeout detection
    #[arg(long, default_value = "10")]
    stale_task_multiplier: u64,

    /// Max retries for summary component (overrides global max if set)
    #[arg(long)]
    summary_max_retries: Option<u32>,

    /// Max retries for abstract component (overrides global max if set)
    #[arg(long)]
    abstract_max_retries: Option<u32>,

    /// Max retries for PDF component (overrides global max if set)
    #[arg(long)]
    pdf_max_retries: Option<u32>,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize tracing
    let level = if args.verbose {
        Level::DEBUG
    } else {
        Level::INFO
    };
    tracing_subscriber::fmt().with_max_level(level).init();

    // Parse backoff strategy
    let backoff_strategy = match args.backoff_strategy.to_lowercase().as_str() {
        "linear" => BackoffStrategy::Linear {
            max_delay_secs: args.max_backoff_delay_secs,
        },
        "exponential" => BackoffStrategy::Exponential {
            max_delay_secs: args.max_backoff_delay_secs,
            jitter: args.backoff_jitter,
        },
        "fibonacci" => BackoffStrategy::Fibonacci {
            max_delay_secs: args.max_backoff_delay_secs,
        },
        _ => BackoffStrategy::Constant, // Default to constant
    };

    // Create retry configuration
    let component_max_retries = if args.summary_max_retries.is_some()
        || args.abstract_max_retries.is_some()
        || args.pdf_max_retries.is_some()
    {
        Some(ComponentRetryConfig {
            summary_max_retries: args.summary_max_retries,
            abstract_max_retries: args.abstract_max_retries,
            pdf_max_retries: args.pdf_max_retries,
        })
    } else {
        None
    };

    let retry_config = RetryConfig {
        empty_queue_sleep_secs: args.empty_queue_sleep_secs,
        stale_task_multiplier: args.stale_task_multiplier,
        backoff_strategy,
        component_max_retries,
    };

    // Create the blueprint
    let blueprint = Blueprint {
        fetcher: Fetcher {
            query: args.query.clone(),
            page_size: args.page_size,
            upload_path_prefix: args.upload_prefix.clone(),
            task_timeout_secs: args.task_timeout_secs,
            max_retry_attempts: args.max_retry_attempts,
            esearch_url: Fetcher::default().esearch_url, // Use default URL
            retry_config,
        },
        connections: Connections {
            db: Database::Postgresql(Postgresql {
                url: args.database_url.clone(),
            }),
            s3_info: S3Info {
                endpoint: args.s3_endpoint.clone(),
                access_key: args.s3_access_key.clone(),
                secret_key: args.s3_secret_key.clone(),
                bucket: args.s3_bucket.clone(),
            },
        },
    };

    // Initialize infrastructure
    let infra_ctx_builder = StdInfraContextBuilder::default()
        .database_url(args.database_url)
        .endpoint(args.s3_endpoint)
        .access_key(args.s3_access_key)
        .secret_key(args.s3_secret_key)
        .bucket(args.s3_bucket)
        .build()?;

    let ctx = infra_ctx_builder.get()?;

    // Execute based on mode
    match args.mode {
        Mode::Sync => {
            // Legacy synchronous mode - fetch and upload immediately
            let multi_progress = Arc::new(MultiProgress::new());
            let main_pb = multi_progress.add(ProgressBar::new_spinner());
            main_pb.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner:.green} [{elapsed_precise}] {msg}")
                    .unwrap(),
            );

            main_pb.set_message(format!("Searching for papers: '{}'", args.query));
            main_pb.enable_steady_tick(std::time::Duration::from_millis(100));

            match fetch_with_progress(&blueprint, ctx, multi_progress.clone()).await {
                Ok(count) => {
                    main_pb.finish_with_message(format!("✓ Successfully processed {} papers", count));
                    Ok(())
                }
                Err(e) => {
                    main_pb.finish_with_message(format!("✗ Failed: {}", e));
                    Err(e.into())
                }
            }
        }
        Mode::Enqueue => {
            // Enqueue tasks and exit
            println!("📋 Enqueueing tasks for query: '{}'", args.query);
            
            match cortexmap_fetcher::enqueue_query(&blueprint, ctx).await {
                Ok(results) => {
                    println!("✓ Successfully enqueued {} tasks", results.len());
                    for (pmc_id, task_id) in &results {
                        // pmc_id already has "PMC" prefix from enqueue_query
                        println!("  - {} (task_id: {})", pmc_id, task_id);
                    }
                    Ok(())
                }
                Err(e) => {
                    eprintln!("✗ Failed to enqueue tasks: {}", e);
                    Err(e.into())
                }
            }
        }
        Mode::Worker => {
            // Run worker loop
            let worker_id = uuid::Uuid::new_v4().to_string();
            tracing::info!(
                "Starting worker {} (timeout: {}s, max retries: {})", 
                worker_id,
                blueprint.fetcher.task_timeout_secs,
                blueprint.fetcher.max_retry_attempts
            );
            
            match cortexmap_fetcher::worker_loop(worker_id, ctx, blueprint).await {
                Ok(()) => Ok(()),
                Err(e) => {
                    tracing::error!("Worker error: {}", e);
                    Err(e.into())
                }
            }
        }
        Mode::Status => {
            // Show queue statistics
            println!("📊 Queue Status\n");
            
            let stats = ctx.infra.get_task_stats().await?;
            println!("Total tasks:      {}", stats.total);
            println!("├─ Pending:       {}", stats.pending);
            println!("├─ In Progress:   {}", stats.in_progress);
            println!("├─ Completed:     {}", stats.completed);
            println!("└─ Failed:        {}", stats.failed);
            Ok(())
        }
    }
}

async fn fetch_with_progress<I>(
    blueprint: &Blueprint,
    ctx: cortexmap_infra::InfraContext<I>,
    multi_progress: Arc<MultiProgress>,
) -> Result<usize, cortexmap_fetcher::FetchError>
where
    I: cortexmap_infra::HttpInfra
        + cortexmap_infra::DatabaseInfra
        + cortexmap_infra::S3Infra
        + Send
        + Sync
        + 'static,
{
    let main_pb = multi_progress.add(ProgressBar::new_spinner());
    main_pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );

    // Fetch metadata
    main_pb.set_message("Fetching metadata...");
    let metadata_collection = cortexmap_fetcher::fetch_metadata(blueprint, ctx.clone()).await?;

    let total = metadata_collection.articles.len();
    main_pb.finish_and_clear();

    if total == 0 {
        let no_results_pb = multi_progress.add(ProgressBar::new_spinner());
        no_results_pb.finish_with_message("⚠ No papers found matching the query");
        return Ok(0);
    }

    // Setup progress bar for downloads
    let download_pb = multi_progress.add(ProgressBar::new(total as u64));
    download_pb.set_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg}",
            )
            .unwrap()
            .progress_chars("#>-"),
    );
    download_pb.set_message("Downloading PDFs...");

    // Fetch PDFs with progress tracking
    let mut pdf_streams = Vec::new();
    for article in &metadata_collection.articles {
        match cortexmap_fetcher::fetch_pdf(article.pmcid.clone(), ctx.clone()).await {
            Ok(stream) => {
                pdf_streams.push(stream);
                download_pb.inc(1);
            }
            Err(e) => {
                tracing::warn!("Failed to fetch PDF for {}: {}", article.pmcid, e);
                download_pb.inc(1);
            }
        }
    }

    download_pb.finish_with_message(format!("Downloaded {} PDFs", pdf_streams.len()));

    // Setup progress bar for uploads
    let upload_count = pdf_streams.len();
    let upload_pb = multi_progress.add(ProgressBar::new(upload_count as u64));
    upload_pb.set_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] [{bar:40.magenta/blue}] {pos}/{len} {msg}",
            )
            .unwrap()
            .progress_chars("#>-"),
    );
    upload_pb.set_message("Uploading to S3...");

    // Create metadata map for quick lookup
    let metadata_map: std::collections::HashMap<_, _> = metadata_collection
        .articles
        .into_iter()
        .map(|article| (article.pmcid.clone(), article.metadata))
        .collect();

    // Upload PDFs and metadata
    let mut successful_uploads = 0;
    for stream in pdf_streams {
        let pdf_key = format!(
            "{}/{}/paper.pdf",
            sterilize_prefix(&blueprint.fetcher.upload_path_prefix),
            stream.pmc_id
        );
        let metadata_key = format!(
            "{}/{}/metadata.json",
            sterilize_prefix(&blueprint.fetcher.upload_path_prefix),
            stream.pmc_id
        );

        // Upload metadata in JSON format
        if let Some(metadata) = metadata_map.get(&stream.pmc_id) {
            let metadata_json = serde_json::to_vec_pretty(metadata).unwrap_or_default();
            
            let metadata_stream = futures::stream::once(async move {
                bytes::Bytes::from(metadata_json)
            });
            
            match ctx
                .infra
                .put_s3(
                    &metadata_key,
                    cortexmap_infra::ContentType::Json,
                    Box::pin(metadata_stream),
                )
                .await
            {
                Ok(()) => {
                    tracing::info!("✓ Uploaded metadata: {}", metadata_key);
                }
                Err(e) => {
                    tracing::warn!("✗ Failed to upload metadata for {}: {}", stream.pmc_id, e);
                }
            }
        }

        // Upload PDF to S3
        let byte_stream = stream
            .stream
            .filter_map(|result| async move { result.ok() });

        match ctx
            .infra
            .put_s3(
                &pdf_key,
                cortexmap_infra::ContentType::Pdf,
                Box::pin(byte_stream),
            )
            .await
        {
            Ok(()) => {
                // Insert into database (minimal record as index)
                match ctx
                    .infra
                    .insert_paper(cortexmap_infra::NewPaper {
                        pmc_id: stream.pmc_id.clone(),
                        s3_url: pdf_key.clone(),
                        uid: uuid::Uuid::new_v4().to_string(),
                        query: blueprint.fetcher.query.clone(),
                    })
                    .await
                {
                    Ok(_paper) => {
                        tracing::info!("✓ Uploaded PDF and indexed: {}", stream.pmc_id);
                        successful_uploads += 1;
                    }
                    Err(e) => {
                        tracing::warn!("✗ Failed to insert into DB for {}: {}", stream.pmc_id, e);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("✗ Failed to upload PDF {}: {}", stream.pmc_id, e);
            }
        }
        upload_pb.inc(1);
    }

    upload_pb.finish_with_message(format!("Uploaded {} PDFs to S3", successful_uploads));

    Ok(successful_uploads)
}

// Helper function from the fetcher crate
fn sterilize_prefix<T: ToString>(prefix: T) -> String {
    let prefix = prefix.to_string();
    prefix
        .split('/')
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}
