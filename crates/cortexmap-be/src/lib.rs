pub mod server;
pub mod worker_manager;

// Re-export generated protobuf types
pub mod proto {
    tonic::include_proto!("queue");
}
