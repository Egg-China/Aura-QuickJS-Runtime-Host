use crate::path_policy::normalize_descriptor_path;
use crate::{HostError, HostResult, PackagePathPolicy};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_DESCRIPTOR_BYTES: u64 = 1024 * 1024;

/// Strict schema-v1 JavaScript payload descriptor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayloadDescriptor {
    module: PathBuf,
}

impl PayloadDescriptor {
    /// Reads and validates a descriptor and its root ES module.
    pub fn read(package_root: &Path, entrypoint: &str) -> HostResult<Self> {
        let policy = PackagePathPolicy::new(package_root)?;
        let entrypoint = normalize_descriptor_path(entrypoint)?;
        if entrypoint.file_name().and_then(|value| value.to_str()) != Some("aura-javascript.json") {
            return Err(HostError::new(
                "invalid-descriptor",
                "entrypoint must name aura-javascript.json",
            ));
        }
        let path = policy.resolve_descriptor_file(
            entrypoint
                .to_str()
                .ok_or_else(|| HostError::new("invalid-descriptor", "entrypoint is not UTF-8"))?,
            "json",
        )?;
        let metadata = fs::metadata(&path).map_err(|error| {
            HostError::new(
                "invalid-descriptor",
                format!("descriptor is unavailable: {error}"),
            )
        })?;
        if !metadata.is_file() || metadata.len() > MAX_DESCRIPTOR_BYTES {
            return Err(HostError::new(
                "invalid-descriptor",
                "descriptor is not a bounded regular file",
            ));
        }
        let bytes = fs::read(&path).map_err(|error| {
            HostError::new(
                "invalid-descriptor",
                format!("descriptor cannot be read: {error}"),
            )
        })?;
        let raw: RawDescriptor = serde_json::from_slice(&bytes).map_err(|error| {
            HostError::new(
                "invalid-descriptor",
                format!("descriptor JSON is invalid: {error}"),
            )
        })?;
        if raw.schema_version != 1 {
            return Err(HostError::new(
                "invalid-descriptor",
                "descriptor schemaVersion must be 1",
            ));
        }
        let module = normalize_descriptor_path(&raw.module)?;
        policy.resolve_descriptor_file(&raw.module, "mjs")?;
        Ok(Self { module })
    }

    /// Returns the validated package-relative root module.
    #[must_use]
    pub fn module(&self) -> &Path {
        &self.module
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDescriptor {
    #[serde(rename = "schemaVersion")]
    schema_version: i64,
    module: String,
}
