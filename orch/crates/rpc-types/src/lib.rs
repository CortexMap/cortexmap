pub mod proto {
    tonic::include_proto!("com.cortexmap.orch");
}

pub use prost_types;

pub use proto::*;
