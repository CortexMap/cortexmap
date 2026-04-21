// Re-export rpc-types for convenience
pub use rpc_types;

pub mod boolean_query;
mod cost;
mod evals;
mod hash;
mod processing;
mod tool_calling;
mod usage;

pub use boolean_query::BooleanQuery;
pub use cost::{
    LlmPricing, LlmProvider, NewLlmCallUsage, UsageAggregate, UsageAggregateFilter,
    UsageByCallerTag, UsageByModel, UsageContext,
};
pub use evals::*;
pub use hash::compute_hash;
pub use processing::*;
pub use tool_calling::*;
pub use usage::{LlmCallOutcome, LlmEndpointKind, Usage};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// RGB color representation matching the database schema
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

impl From<rpc_types::Rgb> for Rgb {
    fn from(proto: rpc_types::Rgb) -> Self {
        Self {
            r: proto.r.clamp(0, 255) as u8,
            g: proto.g.clamp(0, 255) as u8,
            b: proto.b.clamp(0, 255) as u8,
        }
    }
}

impl From<Rgb> for rpc_types::Rgb {
    fn from(rgb: Rgb) -> Self {
        Self {
            r: rgb.r as i32,
            g: rgb.g as i32,
            b: rgb.b as i32,
        }
    }
}

/// Status enum matching the proto definition
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Status {
    FetcherQueue,
    Fetching,
    IndexQueue,
    Indexing,
    Generating,
    Done,
}

impl From<rpc_types::Status> for Status {
    fn from(proto: rpc_types::Status) -> Self {
        match proto {
            rpc_types::Status::FetcherQueue => Status::FetcherQueue,
            rpc_types::Status::Fetching => Status::Fetching,
            rpc_types::Status::IndexQueue => Status::IndexQueue,
            rpc_types::Status::Indexing => Status::Indexing,
            rpc_types::Status::Generating => Status::Generating,
            rpc_types::Status::Done => Status::Done,
        }
    }
}

impl From<Status> for rpc_types::Status {
    fn from(status: Status) -> Self {
        match status {
            Status::FetcherQueue => rpc_types::Status::FetcherQueue,
            Status::Fetching => rpc_types::Status::Fetching,
            Status::IndexQueue => rpc_types::Status::IndexQueue,
            Status::Indexing => rpc_types::Status::Indexing,
            Status::Generating => rpc_types::Status::Generating,
            Status::Done => rpc_types::Status::Done,
        }
    }
}

/// Domain model for RegionMapping matching the database schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionMapping {
    pub id: Uuid,
    pub region_id: i32,
    pub name: String,
    pub acronym: Option<String>,
    pub color: Option<Rgb>,
    pub structure_order: Option<i32>,
    pub parent_region_id: Option<i32>,
    pub parent_acronym: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl RegionMapping {
    /// Create a new RegionMapping with required fields
    pub fn new(region_id: i32, name: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            region_id,
            name,
            acronym: None,
            color: None,
            structure_order: None,
            parent_region_id: None,
            parent_acronym: None,
            created_at: Utc::now(),
        }
    }

    /// Builder pattern for optional fields
    pub fn with_acronym(mut self, acronym: String) -> Self {
        self.acronym = Some(acronym);
        self
    }

    pub fn with_color(mut self, color: Rgb) -> Self {
        self.color = Some(color);
        self
    }

    pub fn with_structure_order(mut self, order: i32) -> Self {
        self.structure_order = Some(order);
        self
    }

    pub fn with_parent(mut self, parent_region_id: i32, parent_acronym: Option<String>) -> Self {
        self.parent_region_id = Some(parent_region_id);
        self.parent_acronym = parent_acronym;
        self
    }
}

impl From<RegionMapping> for rpc_types::RegionMapping {
    fn from(region: RegionMapping) -> Self {
        Self {
            id: Some(rpc_types::Uuid {
                value: region.id.to_string(),
            }),
            region_id: Some(rpc_types::RegionId {
                id: region.region_id,
            }),
            status: rpc_types::Status::FetcherQueue as i32, // Default status
            color: region.color.map(|c| c.into()),
            structure_order: region.structure_order.unwrap_or(0),
            parent_region_id: region.parent_region_id.map(|id| rpc_types::RegionId { id }),
            acronym: region.acronym.map(|a| rpc_types::Acronym { acronym: a }),
            parent_acronym: region
                .parent_acronym
                .map(|a| rpc_types::Acronym { acronym: a }),
        }
    }
}

/// Domain model for BrainRegionEntry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainRegionEntry {
    pub region_id: i32,
    pub name: String,
    pub acronym: String,
    pub summary: String,
    pub created_at: DateTime<Utc>,
}

impl BrainRegionEntry {
    pub fn new(region_id: i32, name: String, acronym: String, summary: String) -> Self {
        Self {
            region_id,
            name,
            acronym,
            summary,
            created_at: Utc::now(),
        }
    }
}

impl From<BrainRegionEntry> for rpc_types::BrainRegionEntry {
    fn from(entry: BrainRegionEntry) -> Self {
        use rpc_types::proto::RegionId;
        Self {
            region_id: Some(RegionId {
                id: entry.region_id,
            }),
            name: entry.name,
            acronym: entry.acronym,
            summary: entry.summary,
            created_at: entry.created_at.to_rfc3339(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_rgb_conversion() {
        let rgb = Rgb::new(255, 128, 0);
        let proto_rgb: rpc_types::Rgb = rgb.into();
        assert_eq!(proto_rgb.r, 255);
        assert_eq!(proto_rgb.g, 128);
        assert_eq!(proto_rgb.b, 0);

        let back: Rgb = proto_rgb.into();
        assert_eq!(back, rgb);
    }

    #[test]
    fn test_region_mapping_builder() {
        let region = RegionMapping::new(1, "Primary Visual Cortex".to_string())
            .with_acronym("V1".to_string())
            .with_color(Rgb::new(255, 0, 0))
            .with_structure_order(100)
            .with_parent(0, Some("Root".to_string()));

        assert_eq!(region.region_id, 1);
        assert_eq!(region.name, "Primary Visual Cortex");
        assert_eq!(region.acronym, Some("V1".to_string()));
        assert_eq!(region.color, Some(Rgb::new(255, 0, 0)));
        assert_eq!(region.structure_order, Some(100));
        assert_eq!(region.parent_region_id, Some(0));
    }

    #[test]
    fn test_status_conversion() {
        let status = Status::Fetching;
        let proto_status: rpc_types::Status = status.into();
        assert_eq!(proto_status, rpc_types::Status::Fetching);

        let back: Status = proto_status.into();
        assert_eq!(back, status);
    }

    #[test]
    fn test_rgb_from_proto_clamps_out_of_range_values() {
        let proto_rgb = rpc_types::Rgb {
            r: -10,
            g: 42,
            b: 999,
        };

        let rgb: Rgb = proto_rgb.into();
        assert_eq!(rgb, Rgb::new(0, 42, 255));
    }

    #[test]
    fn test_region_mapping_proto_conversion_preserves_optional_fields() {
        let created_at = Utc.with_ymd_and_hms(2025, 1, 15, 9, 30, 0).unwrap();
        let region = RegionMapping {
            id: Uuid::parse_str("123e4567-e89b-12d3-a456-426614174000").unwrap(),
            region_id: 42,
            name: "Primary Visual Cortex".to_string(),
            acronym: Some("V1".to_string()),
            color: Some(Rgb::new(12, 34, 56)),
            structure_order: Some(7),
            parent_region_id: Some(5),
            parent_acronym: Some("CTX".to_string()),
            created_at,
        };

        let proto: rpc_types::RegionMapping = region.into();
        let color = proto.color.unwrap();
        assert_eq!(
            proto.id.unwrap().value,
            "123e4567-e89b-12d3-a456-426614174000"
        );
        assert_eq!(proto.region_id.unwrap().id, 42);
        assert_eq!(proto.status, rpc_types::Status::FetcherQueue as i32);
        assert_eq!(color.r, 12);
        assert_eq!(color.g, 34);
        assert_eq!(color.b, 56);
        assert_eq!(proto.structure_order, 7);
        assert_eq!(proto.parent_region_id.unwrap().id, 5);
        assert_eq!(proto.acronym.unwrap().acronym, "V1");
        assert_eq!(proto.parent_acronym.unwrap().acronym, "CTX");
    }

    #[test]
    fn test_region_mapping_proto_conversion_applies_default_values() {
        let region = RegionMapping {
            id: Uuid::nil(),
            region_id: 9,
            name: "Thalamus".to_string(),
            acronym: None,
            color: None,
            structure_order: None,
            parent_region_id: None,
            parent_acronym: None,
            created_at: Utc.with_ymd_and_hms(2025, 2, 1, 0, 0, 0).unwrap(),
        };

        let proto: rpc_types::RegionMapping = region.into();
        assert_eq!(proto.id.unwrap().value, Uuid::nil().to_string());
        assert_eq!(proto.region_id.unwrap().id, 9);
        assert_eq!(proto.status, rpc_types::Status::FetcherQueue as i32);
        assert!(proto.color.is_none());
        assert_eq!(proto.structure_order, 0);
        assert!(proto.parent_region_id.is_none());
        assert!(proto.acronym.is_none());
        assert!(proto.parent_acronym.is_none());
    }

    #[test]
    fn test_brain_region_entry_proto_conversion_formats_created_at_as_rfc3339() {
        let created_at = Utc.with_ymd_and_hms(2024, 6, 10, 14, 5, 30).unwrap();
        let entry = BrainRegionEntry {
            region_id: 101,
            name: "Hippocampus".to_string(),
            acronym: "HPC".to_string(),
            summary: "Supports memory formation".to_string(),
            created_at,
        };

        let proto: rpc_types::BrainRegionEntry = entry.into();
        assert_eq!(proto.region_id.unwrap().id, 101);
        assert_eq!(proto.name, "Hippocampus");
        assert_eq!(proto.acronym, "HPC");
        assert_eq!(proto.summary, "Supports memory formation");
        assert_eq!(proto.created_at, created_at.to_rfc3339());
    }
}
