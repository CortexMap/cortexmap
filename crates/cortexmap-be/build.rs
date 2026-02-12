fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]")
        .type_attribute(".", "#[serde(default)]")
        .compile_protos(&["proto/queue.proto"], &["proto"])?;
    Ok(())
}
