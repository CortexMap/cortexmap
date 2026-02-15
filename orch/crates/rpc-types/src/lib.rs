// Re-export generated protobuf types
pub mod proto {
    tonic::include_proto!("com.cortexmap");
}

// Re-export prost-types for convenience
pub use prost_types;

// Re-export commonly used types for convenience
pub use proto::{
    brain_region_service_client::BrainRegionServiceClient,
    brain_region_service_server::{BrainRegionService, BrainRegionServiceServer},
    Acronym, BrainRegionEntry, BrainRegionList, BrainRegionListResponse, ProcessRegionRequest,
    ProcessRegionResponse, RegionId, RegionMapping, Rgb, SearchBrainRegionRequest,
    SearchBrainRegionResponse, Status, StatusRequest, StatusResponse, Uuid,
};
