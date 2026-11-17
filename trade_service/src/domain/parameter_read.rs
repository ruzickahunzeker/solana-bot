#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParameterReadFailureKind {
    Unavailable,
    Invalid,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ParameterReadFailure {
    kind: ParameterReadFailureKind,
    message: &'static str,
}

impl ParameterReadFailure {
    pub(crate) fn new(kind: ParameterReadFailureKind, message: &'static str) -> Self {
        Self { kind, message }
    }

    pub(crate) fn allows_lane_fallback(&self) -> bool {
        self.kind == ParameterReadFailureKind::Unavailable
    }
}

impl std::fmt::Display for ParameterReadFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message)
    }
}
impl std::error::Error for ParameterReadFailure {}
