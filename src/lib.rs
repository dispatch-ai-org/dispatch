pub mod coherence;
pub mod config;
pub mod db;
pub mod executor;
pub mod follow;
pub mod harness;
pub mod launch;
pub mod lock;
pub mod models;
pub mod orchestrator;
pub mod presenter;
pub mod process;
pub mod reviewer;
pub mod setup;
pub mod source;
pub mod state;

pub use config::Config;
pub use models::*;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
