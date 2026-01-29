use anyhow::Result;
use clap::Parser;
use cortexmap_core::blueprint::connections::{Connections, Database, Fetcher, Postgresql, S3Info};
use cortexmap_core::blueprint::Blueprint;
use futures::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::sync::Arc;
use std_infra::StdInfraContextBuilder;
use tracing::Level;

#[derive(Parser, Debug)]
#[command(name = "cortexmap-cli")]
#[command(about = "Fetch and store academic papers from Europe PMC", long_about = None)]
struct Args {
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

    // Create the blueprint
    let blueprint = Blueprint {
        fetcher: Fetcher {
            query: args.query.clone(),
            page_size: args.page_size,
            upload_path_prefix: args.upload_prefix.clone(),
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

    // Setup progress bars
    let multi_progress = Arc::new(MultiProgress::new());
    let main_pb = multi_progress.add(ProgressBar::new_spinner());
    main_pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} [{elapsed_precise}] {msg}")
            .unwrap(),
    );

    // Fetch metadata
    main_pb.set_message(format!("Searching for papers: '{}'", args.query));
    main_pb.enable_steady_tick(std::time::Duration::from_millis(100));

    // Run the fetcher with progress tracking
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
