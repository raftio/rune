//! `rune-network` — network isolation for the Rune agent runtime.
//!
//! # Concept
//!
//! A **rune-network** is a logical group that agents belong to.  Two agents may
//! only communicate via A2A when they share at least one network.  This is
//! independent of the underlying container / orchestration network.
//!
//! If an agent declares no `networks`, it implicitly belongs to the `"bridge"`
//! network (the default).
//!
//! # Components
//!
//! * [`NetworkPolicy`] — stateless check: do two network lists intersect?
//! * [`NetworkRegistry`] — database-backed resolver that fetches an agent's
//!   networks and checks access, plus locates **direct replica endpoints** to
//!   enable A2A without routing through the central gateway.

pub mod error;
pub mod policy;
pub mod registry;

pub use error::NetworkError;
pub use policy::NetworkPolicy;
pub use registry::NetworkRegistry;
