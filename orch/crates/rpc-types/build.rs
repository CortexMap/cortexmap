fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Apply Serialize + Deserialize to every message and enum in the package.
    let derive = "#[derive(serde::Serialize, serde::Deserialize)]";
    let types: &[&str] = &[];

    let mut config = tonic_prost_build::configure();
    for t in types {
        config = config.type_attribute(t, derive);
    }
    config
        .protoc_arg("--experimental_allow_proto3_optional")
        .compile_protos(&["../../../proto/orch/orch.proto"], &["../../../proto"])?;
    Ok(())
}
