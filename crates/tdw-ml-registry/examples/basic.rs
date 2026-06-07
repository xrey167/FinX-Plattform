//! Offline `tdw-ml-registry` example: register a model, read it back, and show
//! the path-traversal and duplicate rejections. Fully in-memory — no I/O.
//!
//! ```text
//! cargo run --example tdw_ml_registry_basic -p tdw-ml-registry
//! ```

use tdw_ml_registry::{ModelKind, ModelRegistration, ModelRegistry, ModelRegistryError};

fn main() {
    let mut registry = ModelRegistry::default();

    let model = ModelRegistration {
        model_id: "embedding/local-hash".to_string(),
        kind: ModelKind::Embedding,
        version: "0.1.0".to_string(),
        artifact_uri: "s3://models/local-hash".to_string(),
        owner: "tdw-ml-registry".to_string(),
    };

    registry
        .register(model.clone())
        .expect("valid model should register");
    assert_eq!(registry.get("embedding/local-hash"), Some(&model));
    assert_eq!(
        registry.model_ids(),
        vec!["embedding/local-hash".to_string()]
    );
    println!("registered: {:?}", registry.model_ids());

    // A traversal-style id is rejected.
    let traversal = ModelRegistration {
        model_id: "../secret".to_string(),
        ..model.clone()
    };
    assert_eq!(
        registry.register(traversal),
        Err(ModelRegistryError::InvalidModelId)
    );

    // Re-registering the same id is a duplicate.
    assert_eq!(
        registry.register(model),
        Err(ModelRegistryError::DuplicateModel)
    );
    println!("traversal id and duplicate registration are rejected, as expected");
}
