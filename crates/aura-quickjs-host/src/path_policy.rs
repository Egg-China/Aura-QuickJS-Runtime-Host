use crate::{HostError, HostResult};
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Resolves payload files while containing canonical paths beneath one package root.
#[derive(Clone, Debug)]
pub struct PackagePathPolicy {
    package_root: PathBuf,
    canonical_root: PathBuf,
}

impl PackagePathPolicy {
    /// Creates a policy for an existing package directory.
    pub fn new(package_root: &Path) -> HostResult<Self> {
        let canonical_root = fs::canonicalize(package_root).map_err(|error| {
            HostError::new(
                "invalid-descriptor",
                format!("package root is unavailable: {error}"),
            )
        })?;
        if !canonical_root.is_dir() {
            return Err(HostError::new(
                "invalid-descriptor",
                "package root is not a directory",
            ));
        }
        Ok(Self {
            package_root: package_root.to_path_buf(),
            canonical_root,
        })
    }

    /// Resolves one relative ES module specifier beneath the package.
    pub fn resolve_module(&self, specifier: &str, referrer: Option<&Path>) -> HostResult<PathBuf> {
        if !specifier.starts_with("./") && !specifier.starts_with("../") {
            return Err(HostError::new(
                "invalid-module",
                "module specifier must be relative",
            ));
        }
        let base = referrer
            .and_then(Path::parent)
            .unwrap_or_else(|| Path::new(""));
        let relative = normalize_relative(&base.join(specifier), true)?;
        if relative.extension().and_then(|value| value.to_str()) != Some("mjs") {
            return Err(HostError::new(
                "invalid-module",
                "module path must end in .mjs",
            ));
        }
        self.resolve_regular_file(&relative, "invalid-module")
    }

    /// Resolves a strict descriptor-owned package-relative file.
    pub(crate) fn resolve_descriptor_file(
        &self,
        relative: &str,
        extension: &str,
    ) -> HostResult<PathBuf> {
        let normalized = normalize_descriptor_path(relative)?;
        if normalized.extension().and_then(|value| value.to_str()) != Some(extension) {
            return Err(HostError::new(
                "invalid-descriptor",
                format!("payload path must end in .{extension}"),
            ));
        }
        self.resolve_regular_file(&normalized, "invalid-descriptor")
    }

    fn resolve_regular_file(
        &self,
        relative: &Path,
        missing_code: &'static str,
    ) -> HostResult<PathBuf> {
        let joined = self.package_root.join(relative);
        let canonical = fs::canonicalize(&joined).map_err(|error| {
            HostError::new(
                missing_code,
                format!("payload file is unavailable: {error}"),
            )
        })?;
        if !canonical.starts_with(&self.canonical_root) {
            return Err(HostError::new(
                "path-escape",
                "payload path escapes the package root",
            ));
        }
        if !canonical.is_file() {
            return Err(HostError::new(
                missing_code,
                "payload path is not a regular file",
            ));
        }
        Ok(joined)
    }
}

pub(crate) fn normalize_descriptor_path(value: &str) -> HostResult<PathBuf> {
    if value.is_empty()
        || value.contains('\\')
        || value.contains('\0')
        || value.contains(':')
        || value.starts_with('/')
    {
        return Err(HostError::new(
            "path-escape",
            "payload path is not safely relative",
        ));
    }
    normalize_relative(Path::new(value), false)
}

fn normalize_relative(path: &Path, allow_current: bool) -> HostResult<PathBuf> {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => output.push(segment),
            Component::CurDir if allow_current => {}
            Component::ParentDir => {
                if !output.pop() {
                    return Err(HostError::new(
                        "path-escape",
                        "payload path escapes the package root",
                    ));
                }
            }
            _ => {
                return Err(HostError::new(
                    "path-escape",
                    "payload path contains a forbidden segment",
                ));
            }
        }
    }
    if output.as_os_str().is_empty() {
        return Err(HostError::new("path-escape", "payload path is empty"));
    }
    Ok(output)
}
