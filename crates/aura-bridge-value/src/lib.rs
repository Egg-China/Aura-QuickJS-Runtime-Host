#![deny(missing_docs)]

//! Bridge Value v1 types shared by Aura process Hosts.

mod error;
mod value;

pub use error::{BridgeErrorKind, Error, ErrorCode};
pub use value::{HandleValue, Value};
