use crate::{Contract, ContractKind, ContractStatus};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContractConfig {
    pub schema_version: u32,
    pub functions: Vec<FunctionContract>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FunctionContract {
    pub function: String,
    #[serde(default)]
    pub instance: Option<String>,
    #[serde(default)]
    pub arguments: Vec<String>,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub ensures: Vec<String>,
    #[serde(default)]
    pub trusted: bool,
    #[serde(default)]
    pub no_panic: bool,
    #[serde(default)]
    pub modifies: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returns_alias: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

impl ContractConfig {
    pub fn read(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path)
            .map_err(|error| format!("contracts {}: {error}", path.display()))?;
        let config: Self = serde_json::from_slice(&bytes)
            .map_err(|error| format!("contracts {}: {error}", path.display()))?;
        if config.schema_version != 1 {
            return Err("contract configuration needs schema_version 1".to_owned());
        }
        let mut selectors = BTreeSet::new();
        for function in &config.functions {
            if !function.function.contains("::")
                || function.function.trim() != function.function
                || function.function.contains('*')
                || function.function.contains('?')
            {
                return Err("contract functions require exact crate-qualified paths".to_owned());
            }
            if !selectors.insert((&function.function, &function.instance)) {
                return Err("duplicate function/instance contract selector".to_owned());
            }
            let mut names = BTreeSet::new();
            for name in &function.arguments {
                if syn::parse_str::<syn::Ident>(name).is_err()
                    || name == "result"
                    || name.starts_with("final_")
                    || !names.insert(name)
                {
                    return Err(
                        "argument aliases must be unique, unreserved identifiers".to_owned()
                    );
                }
            }
            for predicate in function.requires.iter().chain(&function.ensures) {
                syn::parse_str::<syn::Expr>(predicate)
                    .map_err(|error| format!("invalid contract predicate: {error}"))?;
            }
            if function.trusted {
                if !function.no_panic
                    || function
                        .reason
                        .as_ref()
                        .is_none_or(|reason| reason.trim().is_empty())
                {
                    return Err(
                        "trusted functions require no_panic=true and a nonempty reason".to_owned(),
                    );
                }
                if let Some(modifies) = &function.modifies
                    && modifies
                        .iter()
                        .any(|name| !function.arguments.contains(name))
                {
                    return Err("modifies must name declared positional arguments".to_owned());
                }
                if let Some(name) = &function.returns_alias
                    && !function.arguments.contains(name)
                {
                    return Err("returns_alias must name a declared positional argument".to_owned());
                }
            } else if function.modifies.is_some()
                || function.returns_alias.is_some()
                || function.reason.is_some()
            {
                return Err(
                    "effects, return aliases and reasons apply only to trusted summaries"
                        .to_owned(),
                );
            }
        }
        Ok(config)
    }
}

impl FunctionContract {
    pub fn selector(&self) -> String {
        self.instance.as_ref().map_or_else(
            || self.function.clone(),
            |instance| format!("{} {instance}", self.function),
        )
    }
    pub fn metadata(&self) -> Vec<Contract> {
        let mut result = Vec::new();
        if self.no_panic {
            result.push(Contract {
                kind: ContractKind::NoPanic,
                predicate: None,
                status: ContractStatus::PendingVerification,
            });
        }
        for (kind, predicates) in [
            (ContractKind::Requires, &self.requires),
            (ContractKind::Ensures, &self.ensures),
        ] {
            for predicate in predicates {
                result.push(Contract {
                    kind: kind.clone(),
                    predicate: Some(predicate.clone()),
                    status: ContractStatus::PendingVerification,
                });
            }
        }
        result
    }
}
