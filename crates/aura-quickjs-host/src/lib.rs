#![deny(missing_docs)]

//! Isolated QuickJS Host process implementation.

mod descriptor;
mod error;
mod path_policy;
mod server;

pub use descriptor::PayloadDescriptor;
pub use error::{HostError, HostResult};
pub use path_policy::PackagePathPolicy;
pub use server::{GuestEngine, ProcessServer};
