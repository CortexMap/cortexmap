# CortexMap

A Rust-based brain atlas data management and query system designed for neuroscience research. CortexMap provides a powerful query engine for searching and retrieving neuroscientific data from various sources.

## 🚀 Quick Start - Frontend

```bash
cd fetcher_fe
npm install
npm start
```

Access the frontend at http://localhost:3000/fetcher-fe

**Backend**: https://capstone.ssdd.dev/fetcher-be

See `fetcher_fe/README.md` for complete frontend documentation.

## Features

- **Advanced Query System**: Comprehensive boolean query support with nested logic
- **YAML Configuration**: Declarative configuration for complex queries
- **Modular Architecture**: Clean separation between core logic and data fetching
- **Serde Integration**: Full serialization/deserialization support
- **PubMed Integration**: Built-in support for fetching neuroscience literature

## Architecture

CortexMap is organized as a Rust workspace with the following crates:

### Core Components

#### `cortexmap-core`
The core library providing:
- Configuration management (`config`)
- Query engine with boolean operations (`BooleanQuery`)
- Blueprint definitions for data fetching (`blueprint`)

#### `cortexmap-fetcher`
Data fetching functionality:
- PubMed/EUtils integration (`fetch/metadata`)
- Error handling with `thiserror`
- Extensible fetcher architecture

## Query System

CortexMap supports sophisticated boolean queries including:

- **Term Queries**: Simple text search
- **Phrase Queries**: Exact phrase matching
- **Field Queries**: Search in specific fields with optional boost
- **Wildcard Queries**: Pattern matching with `*` and `?`
- **Boolean Operations**: AND, OR, NOT with nested logic
- **Range Queries**: Numeric and date range filtering
- **Boost Queries**: Relevance scoring adjustment

### Example Configuration

```yaml
query: !and
  - !or
    - !term "rust"
    - !term "go"
  - !field
    name: "category"
    value: "programming"
  - !not
    query: !or
      - !term "deprecated"
      - !term "legacy"
  - !boost
    query: !phrase "best practices"
    factor: 2.5
```

## Getting Started

### Prerequisites

- Rust 2024 edition or later
- Cargo package manager

### Installation

Clone the repository and build the project:

```bash
git clone <repository-url>
cd cortexmap
cargo build --all
```

### Running Tests

```bash
cargo test
```

Note: Some test fixtures may need to be created for full test coverage.

## Usage

### Basic Query Example

```rust
use cortexmap_core::config::{Config, BooleanQuery};

// Create a simple term query
let query = BooleanQuery::term("neuroscience");

// Create a complex nested query
let complex_query = BooleanQuery::and(vec![
    BooleanQuery::or(vec![
        BooleanQuery::term("fMRI"),
        BooleanQuery::term("optogenetics")
    ]),
    BooleanQuery::field("species", "mouse"),
    BooleanQuery::not(BooleanQuery::term("review"))
]);

// Convert to query string
let query_string = complex_query.to_string();
```

### Configuration from YAML

```rust
use cortexmap_core::config::Config;

let yaml_config = r#"
query: !and
  - !term "motor cortex"
  - !field
    name: "species"
    value: "human"
"#;

let config = Config::from_yaml(yaml_config)?;
```

## Development Status

This is an active development project. Current status:

- ✅ Core query engine with comprehensive boolean operations
- ✅ YAML configuration system with extensive test coverage
- ✅ Modular architecture with clear separation of concerns
- 🚧 PubMed fetcher implementation (in progress)
- 🚧 Error handling improvements
- 📋 Documentation and examples

## Testing

The project includes comprehensive tests for the query system:

```bash
# Run all tests
cargo test

# Run tests for specific crate
cargo test -p cortexmap-core

# Run with output
cargo test -- --nocapture
```

Test fixtures are located in `crates/cortexmap-core/src/fixtures/` and cover various query patterns including:
- Simple term and phrase queries
- Complex nested boolean operations
- Field-specific queries with boost factors
- Range queries and wildcard patterns

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests for new functionality
5. Ensure all tests pass
6. Submit a pull request

## License

This project is part of the Brain Atlas Capstone project.

## Dependencies

- **serde**: Serialization/deserialization framework
- **serde_yaml**: YAML support for configuration
- **thiserror**: Error handling
- **urlencoding**: URL encoding for web requests

## Future Roadmap

- [ ] Complete PubMed/EUtils fetcher implementation
- [ ] Add support for additional data sources
- [ ] Implement caching mechanisms
- [ ] Add async/await support for data fetching
- [ ] Create CLI interface
- [ ] Add visualization capabilities
- [ ] Performance optimization for large datasets