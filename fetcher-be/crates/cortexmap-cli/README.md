# CortexMap CLI

A command-line tool for fetching and storing academic papers from Europe PMC with real-time progress tracking.

## Features

- 🔍 Search Europe PMC database
- 📥 Download PDFs automatically
- 📦 Upload to S3-compatible storage
- 💾 Store metadata in PostgreSQL
- 📊 Real-time progress bars for each operation
- 🎨 Colorful, informative output

## Installation

Build the CLI from the workspace root:

```bash
cargo build --release -p cortexmap-cli
```

The binary will be available at `target/release/cortexmap-cli`.

## Usage

### Basic Usage

```bash
cortexmap-cli \
  --query "cancer research" \
  --page-size 20 \
  --database-url "postgres://user:pass@localhost/cortexmap" \
  --s3-endpoint "http://localhost:9000" \
  --s3-access-key "minioadmin" \
  --s3-secret-key "minioadmin" \
  --s3-bucket "papers"
```

### Using Environment Variables

Set the following environment variables to avoid passing credentials on the command line:

```bash
export DATABASE_URL="postgres://user:pass@localhost/cortexmap"
export S3_ENDPOINT="http://localhost:9000"
export S3_ACCESS_KEY="minioadmin"
export S3_SECRET_KEY="minioadmin"
export S3_BUCKET="papers"
```

Then run:

```bash
cortexmap-cli --query "cancer research" --page-size 20
```

### Options

- `-q, --query <QUERY>` - Search query for Europe PMC (required)
- `-n, --page-size <PAGE_SIZE>` - Number of results to fetch (default: 10)
- `-u, --upload-prefix <UPLOAD_PREFIX>` - S3 upload path prefix (default: papers)
- `--database-url <DATABASE_URL>` - PostgreSQL connection URL (or set `DATABASE_URL` env var)
- `--s3-endpoint <S3_ENDPOINT>` - S3 endpoint URL (or set `S3_ENDPOINT` env var)
- `--s3-access-key <S3_ACCESS_KEY>` - S3 access key (or set `S3_ACCESS_KEY` env var)
- `--s3-secret-key <S3_SECRET_KEY>` - S3 secret key (or set `S3_SECRET_KEY` env var)
- `--s3-bucket <S3_BUCKET>` - S3 bucket name (or set `S3_BUCKET` env var)
- `-v, --verbose` - Enable verbose logging

## Example Output

```
⠁ [00:00:02] Searching for papers: 'cancer research'
⠒ Fetching metadata...
⠉ [00:00:05] [########################################] 20/20 Downloaded 18 PDFs
⠉ [00:00:12] [########################################] 18/18 Uploaded 18 papers to S3
✓ Successfully processed 18 papers
```

## Requirements

- PostgreSQL database
- S3-compatible storage (MinIO, AWS S3, etc.)
- Internet connection for accessing Europe PMC

## Architecture

The CLI uses the `cortexmap-fetcher` library which:
1. Queries Europe PMC API for metadata
2. Fetches PDF files for papers with PMC IDs
3. Uploads PDFs to S3 storage
4. Stores paper metadata in PostgreSQL

Progress is tracked using the `indicatif` crate with separate progress bars for:
- Metadata fetching
- PDF downloads
- S3 uploads
