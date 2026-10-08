use serde::{Deserialize, Serialize};

pub const HELP: &str = "Analysis limits: --max-steps N (8192), --max-call-depth N (16),\n\
    --max-query-bytes N (200000), --root-timeout-secs N (30),\n\
    --solver-timeout-ms N (5000). Values must be positive.\n\
    Raising limits never treats unfinished or unsupported analysis as a proof.";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct AnalysisLimits {
    pub max_steps: usize,
    pub max_call_depth: usize,
    pub max_query_bytes: usize,
    pub root_timeout_secs: u64,
    pub solver_timeout_ms: u32,
}

impl Default for AnalysisLimits {
    fn default() -> Self {
        Self {
            max_steps: 8192,
            max_call_depth: 16,
            max_query_bytes: 200_000,
            root_timeout_secs: 30,
            solver_timeout_ms: 5000,
        }
    }
}

#[derive(Clone, Copy)]
pub enum LimitOption {
    Steps,
    CallDepth,
    QueryBytes,
    RootTimeout,
    SolverTimeout,
}

impl LimitOption {
    pub fn parse(argument: &str) -> Option<(Self, Option<&str>)> {
        let (flag, value) = match argument.split_once('=') {
            Some((flag, value)) => (flag, Some(value)),
            None => (argument, None),
        };
        let option = match flag {
            "--max-steps" => Self::Steps,
            "--max-call-depth" => Self::CallDepth,
            "--max-query-bytes" => Self::QueryBytes,
            "--root-timeout-secs" => Self::RootTimeout,
            "--solver-timeout-ms" => Self::SolverTimeout,
            _ => return None,
        };
        Some((option, value))
    }

    pub fn flag(self) -> &'static str {
        match self {
            Self::Steps => "--max-steps",
            Self::CallDepth => "--max-call-depth",
            Self::QueryBytes => "--max-query-bytes",
            Self::RootTimeout => "--root-timeout-secs",
            Self::SolverTimeout => "--solver-timeout-ms",
        }
    }

    pub fn apply(self, limits: &mut AnalysisLimits, value: &str) -> Result<(), String> {
        let number: u64 = value
            .parse()
            .ok()
            .filter(|number| *number > 0)
            .ok_or_else(|| format!("{} requires a positive integer", self.flag()))?;
        let overflow = || format!("{} exceeds its supported integer range", self.flag());
        match self {
            Self::Steps => limits.max_steps = number.try_into().map_err(|_| overflow())?,
            Self::CallDepth => limits.max_call_depth = number.try_into().map_err(|_| overflow())?,
            Self::QueryBytes => {
                limits.max_query_bytes = number.try_into().map_err(|_| overflow())?
            }
            Self::RootTimeout => limits.root_timeout_secs = number,
            Self::SolverTimeout => {
                limits.solver_timeout_ms = number.try_into().map_err(|_| overflow())?;
            }
        }
        limits.validate()
    }
}

impl AnalysisLimits {
    pub fn validate(self) -> Result<(), String> {
        if self.max_steps == 0
            || self.max_call_depth == 0
            || self.max_query_bytes == 0
            || self.root_timeout_secs == 0
            || self.solver_timeout_ms == 0
        {
            return Err("analysis limits must all be positive".to_owned());
        }
        if std::time::Instant::now()
            .checked_add(std::time::Duration::from_secs(self.root_timeout_secs))
            .is_none()
        {
            return Err("root timeout exceeds the host clock range".to_owned());
        }
        Ok(())
    }

    pub fn from_environment() -> Result<Self, String> {
        let limits = match std::env::var("MIREN_LIMITS") {
            Ok(value) => serde_json::from_str(&value)
                .map_err(|error| format!("invalid MIREN_LIMITS: {error}"))?,
            Err(std::env::VarError::NotPresent) => Self::default(),
            Err(error) => return Err(format!("invalid MIREN_LIMITS: {error}")),
        };
        Self::validate(limits)?;
        Ok(limits)
    }

    pub fn solver_host_timeout(self) -> std::time::Duration {
        std::time::Duration::from_millis(u64::from(self.solver_timeout_ms) + 1000)
    }
}
