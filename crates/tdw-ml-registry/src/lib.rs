#![forbid(unsafe_code)]

use std::collections::BTreeMap;

pub const CRATE_NAME: &str = "tdw-ml-registry";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelKind {
    Embedding,
    LanguageModel,
    Classifier,
    Reranker,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelRegistration {
    pub model_id: String,
    pub kind: ModelKind,
    pub version: String,
    pub artifact_uri: String,
    pub owner: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelRegistryError {
    InvalidModelId,
    InvalidVersion,
    InvalidArtifactUri,
    InvalidOwner,
    DuplicateModel,
}

#[derive(Clone, Debug, Default)]
pub struct ModelRegistry {
    models: BTreeMap<String, ModelRegistration>,
}

impl ModelRegistry {
    pub fn register(&mut self, model: ModelRegistration) -> Result<(), ModelRegistryError> {
        validate_registration(&model)?;
        if self.models.contains_key(&model.model_id) {
            return Err(ModelRegistryError::DuplicateModel);
        }
        self.models.insert(model.model_id.clone(), model);
        Ok(())
    }

    pub fn get(&self, model_id: &str) -> Option<&ModelRegistration> {
        self.models.get(model_id)
    }

    pub fn model_ids(&self) -> Vec<String> {
        self.models.keys().cloned().collect()
    }
}

pub fn validate_registration(model: &ModelRegistration) -> Result<(), ModelRegistryError> {
    if !is_model_id(&model.model_id) {
        return Err(ModelRegistryError::InvalidModelId);
    }
    if model.version.trim().is_empty() || model.version.chars().any(char::is_control) {
        return Err(ModelRegistryError::InvalidVersion);
    }
    if !(model.artifact_uri.starts_with("s3://")
        || model.artifact_uri.starts_with("https://")
        || model.artifact_uri.starts_with("file://"))
        || model.artifact_uri.contains("..")
        || model.artifact_uri.chars().any(char::is_control)
    {
        return Err(ModelRegistryError::InvalidArtifactUri);
    }
    if model.owner.trim().is_empty() || model.owner.chars().any(char::is_control) {
        return Err(ModelRegistryError::InvalidOwner);
    }
    Ok(())
}

fn is_model_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | '/')
        })
        && !value.contains("//")
        && !value.split('/').any(|part| matches!(part, "." | ".." | ""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_valid_model_contract() {
        let mut registry = ModelRegistry::default();
        let model = ModelRegistration {
            model_id: "embedding/local-hash".to_string(),
            kind: ModelKind::Embedding,
            version: "0.1.0".to_string(),
            artifact_uri: "s3://models/local-hash".to_string(),
            owner: CRATE_NAME.to_string(),
        };

        assert_eq!(registry.register(model.clone()), Ok(()));
        assert_eq!(registry.get("embedding/local-hash"), Some(&model));
        assert_eq!(
            registry.model_ids(),
            vec!["embedding/local-hash".to_string()]
        );
    }

    #[test]
    fn rejects_path_traversal_and_duplicate_models() {
        let mut registry = ModelRegistry::default();
        let model = ModelRegistration {
            model_id: "../secret".to_string(),
            kind: ModelKind::Embedding,
            version: "0.1.0".to_string(),
            artifact_uri: "s3://models/local-hash".to_string(),
            owner: CRATE_NAME.to_string(),
        };

        assert_eq!(
            registry.register(model.clone()),
            Err(ModelRegistryError::InvalidModelId)
        );
        let valid = ModelRegistration {
            model_id: "embedding/local-hash".to_string(),
            ..model
        };
        assert_eq!(registry.register(valid.clone()), Ok(()));
        assert_eq!(
            registry.register(valid),
            Err(ModelRegistryError::DuplicateModel)
        );
    }
}
