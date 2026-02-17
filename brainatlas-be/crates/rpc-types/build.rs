fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Apply Serialize + Deserialize to every message and enum in the package.
    let derive = "#[derive(serde::Serialize, serde::Deserialize)]";
    let types = [
        "com.cortexmap.RegionID",
        "com.cortexmap.Acronym",
        "com.cortexmap.Uuid",
        "com.cortexmap.SearchBrainRegionRequest",
        "com.cortexmap.BrainRegionList",
        "com.cortexmap.BrainRegionEntry",
        "com.cortexmap.SearchBrainRegionResponse",
        "com.cortexmap.RGB",
        "com.cortexmap.RegionMapping",
        "com.cortexmap.BrainRegionListResponse",
        "com.cortexmap.StatusRequest",
        "com.cortexmap.StatusResponse",
        "com.cortexmap.ProcessRegionRequest",
        "com.cortexmap.ProcessRegionResponse",
        "com.cortexmap.GenerateQueriesRequest",
        "com.cortexmap.GenerateQueriesResponse",
        "com.cortexmap.PaperMetadata",
        "com.cortexmap.Status",
    ];

    let mut config = tonic_prost_build::configure();
    for t in &types {
        config = config.type_attribute(t, derive);
    }
    
    // Add serde(default) to paper_metadata field for backward compatibility
    config = config.field_attribute(
        "com.cortexmap.ProcessRegionRequest.paper_metadata",
        "#[serde(default)]"
    );
    
    config.compile_protos(
        &["../../../proto/llm/brain.proto"],
        &["../../../proto"],
    )?;
    Ok(())
}
