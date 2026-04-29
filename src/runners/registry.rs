//! Stage-runner adapter registry.

use std::{collections::BTreeMap, fmt};

use super::{RunnerError, RunnerResult, StageRunnerAdapter};

type BoxedStageRunnerAdapter = Box<dyn StageRunnerAdapter>;

/// Maps stable runner names to adapter implementations.
#[derive(Default)]
pub struct RunnerRegistry {
    adapters: BTreeMap<String, BoxedStageRunnerAdapter>,
}

impl fmt::Debug for RunnerRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunnerRegistry")
            .field("names", &self.names())
            .finish()
    }
}

impl RunnerRegistry {
    /// Builds an empty runner registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an adapter under a stable runner name.
    pub fn register<A>(&mut self, runner_name: impl Into<String>, adapter: A) -> RunnerResult<()>
    where
        A: StageRunnerAdapter + 'static,
    {
        self.register_boxed(runner_name, Box::new(adapter))
    }

    /// Registers a boxed adapter under a stable runner name.
    pub fn register_boxed(
        &mut self,
        runner_name: impl Into<String>,
        adapter: BoxedStageRunnerAdapter,
    ) -> RunnerResult<()> {
        let runner_name = normalize_runner_name(runner_name.into())?;
        if self.adapters.contains_key(&runner_name) {
            return Err(RunnerError::DuplicateRunner { runner_name });
        }
        self.adapters.insert(runner_name, adapter);
        Ok(())
    }

    /// Returns an adapter by name.
    #[must_use]
    pub fn get(&self, runner_name: &str) -> Option<&dyn StageRunnerAdapter> {
        self.adapters.get(runner_name).map(Box::as_ref)
    }

    /// Returns sorted adapter names.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.adapters.keys().cloned().collect()
    }

    /// Returns true when no adapters are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.adapters.is_empty()
    }
}

pub(crate) fn normalize_runner_name(runner_name: String) -> RunnerResult<String> {
    let trimmed = runner_name.trim();
    if trimmed.is_empty() {
        return Err(RunnerError::InvalidRunnerName {
            message: "runner name is required".to_owned(),
        });
    }
    Ok(trimmed.to_owned())
}
