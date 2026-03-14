pub mod bridge;
pub mod config;
pub mod error;
pub mod formatter;
pub mod registry;
pub mod router;
pub mod telegram;
pub mod types;
pub mod webhook;

pub use bridge::{BridgeManager, ChannelBridgeHandle};
pub use config::*;
pub use error::ChannelError;
pub use registry::{create_adapter, configs_from_env, ChannelConfig, ChannelsFile};
pub use router::AgentRouter;
pub use telegram::TelegramAdapter;
pub use types::*;
