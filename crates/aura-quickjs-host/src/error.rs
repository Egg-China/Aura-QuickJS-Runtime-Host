use std::fmt::{Display, Formatter};

/// Result returned by QuickJS Host validation and execution.
pub type HostResult<T> = Result<T, HostError>;

/// One bounded QuickJS Host failure with a stable external code.
#[derive(Debug)]
pub struct HostError {
    code: &'static str,
    message: String,
}

impl HostError {
    /// Creates a failure with a stable code and bounded diagnostic.
    #[must_use]
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        let mut message = message.into();
        if message.len() > 4096 {
            let mut end = 4096;
            while !message.is_char_boundary(end) {
                end -= 1;
            }
            message.truncate(end);
        }
        Self { code, message }
    }

    /// Returns the stable lower-case kebab error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }

    /// Reports whether continuing could reuse inconsistent runtime state.
    pub(crate) fn is_fatal(&self) -> bool {
        matches!(
            self.code,
            "deadline-exceeded" | "resource-limit" | "runtime-failure"
        )
    }
}

impl Display for HostError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for HostError {}
