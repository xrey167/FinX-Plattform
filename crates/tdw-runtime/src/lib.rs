#![forbid(unsafe_code)]

use tdw_core::{ProviderRegistry, RegistryEntry, Result};

#[derive(Clone, Debug, Default)]
pub struct CommandRunner {
    registry: ProviderRegistry,
}

impl CommandRunner {
    pub fn new(registry: ProviderRegistry) -> Self {
        Self { registry }
    }

    pub fn register_provider(&mut self, entry: RegistryEntry) -> Result<()> {
        self.registry.register(entry)
    }

    pub fn registered_providers(&self) -> &[RegistryEntry] {
        self.registry.entries()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_core::ProviderKind;

    #[test]
    fn runner_exposes_explicit_provider_registration() {
        let mut runner = CommandRunner::default();
        assert!(
            runner
                .register_provider(RegistryEntry {
                    provider: "fileset",
                    endpoint: "equity_historical",
                    kind: ProviderKind::Fetcher,
                })
                .is_ok()
        );

        assert_eq!(runner.registered_providers().len(), 1);
    }
}
