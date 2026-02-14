pub mod server;
pub mod worker_manager;

// Re-export generated protobuf message types
pub mod proto {
    tonic::include_proto!("queue");
}
