//! Configuration schema migrations, run at activation inside the
//! generation's transaction (`.context/extensions-manifest.md` "Private
//! configuration services"). A layer stored at an older schema version is
//! carried to the declared one by the extension's pure Starlark steps and
//! written back with a compare-and-swap; reads of a layer that has not been
//! carried keep failing closed, so nothing of an extension runs on stale
//! settings.

use oxplow_db::extension_config_store::{self, ConfigNamespace, ConfigSnapshot, ConfigStoreError};
use oxplow_domain::extension_state::Scope;
use rusqlite::Connection;
use serde_json::Value;

use crate::extension::extensions::{config_schema::ConfigurationFile, Extension};

/// Carry every enabled extension's stored layers up to its declared schema
/// version. The first failure (a step that errors, returns something that is
/// not an object, or leaves settings the schema refuses) names the
/// extension and the step; the caller's transaction rolls back, so no layer
/// of any extension is left half-migrated.
pub fn migrate_all_tx<'a>(
    conn: &Connection,
    extensions: impl IntoIterator<Item = &'a Extension>,
) -> Result<(), std::collections::BTreeMap<String, Vec<String>>> {
    for extension in extensions {
        if !extension.enabled {
            continue;
        }
        let Some(declaration) = &extension.configuration else {
            continue;
        };
        migrate_tx(conn, extension, declaration).map_err(|problem| {
            std::collections::BTreeMap::from([(extension.name.clone(), vec![problem])])
        })?;
    }
    Ok(())
}

fn migrate_tx(
    conn: &Connection,
    extension: &Extension,
    declaration: &ConfigurationFile,
) -> Result<(), String> {
    let stored = |scope| -> Result<Option<ConfigSnapshot>, String> {
        let namespace = ConfigNamespace::new(extension.name.clone(), scope)
            .map_err(|_| "its name can't hold configuration".to_string())?;
        extension_config_store::read_tx(conn, &namespace).map_err(|error| match error {
            ConfigStoreError::Storage(storage) => storage.to_string(),
            _ => format!("its stored {} settings are unreadable", scope_word(scope)),
        })
    };
    let layers = [
        (Scope::Project, stored(Scope::Project)?),
        (Scope::Personal, stored(Scope::Personal)?),
    ];
    let mut carried: Vec<(Scope, ConfigSnapshot, Value)> = Vec::new();
    let mut effective: [Option<Value>; 2] = [None, None];
    for (i, (scope, layer)) in layers.iter().enumerate() {
        let Some(layer) = layer else {
            continue;
        };
        effective[i] = Some(layer.values().clone());
        let from = layer.stamp().schema_version;
        if from == declaration.version {
            continue;
        }
        let word = scope_word(*scope);
        let path = declaration.migration_path(from).ok_or_else(|| {
            if from > declaration.version {
                format!(
                    "its {word} settings are at schema version {from}, newer than the declared {}",
                    declaration.version
                )
            } else {
                format!(
                    "its {word} settings are at schema version {from} and no migration starts there"
                )
            }
        })?;
        let mut values = layer.values().clone();
        for step in path {
            let script = extension.config_scripts.get(&step.entry).ok_or_else(|| {
                format!(
                    "migration {} -> {}: `{}` isn't loaded",
                    step.from, step.to, step.entry
                )
            })?;
            let moved = oxplow_script::runtime::run_starlark(script, &values).map_err(|error| {
                format!(
                    "{word} settings, migration {} -> {} (`{}`): {error}",
                    step.from, step.to, step.entry
                )
            })?;
            if !moved.is_object() {
                return Err(format!(
                    "{word} settings, migration {} -> {} (`{}`) must return an object",
                    step.from, step.to, step.entry
                ));
            }
            values = moved;
        }
        effective[i] = Some(values.clone());
        carried.push((*scope, layer.clone(), values));
    }
    if carried.is_empty() {
        return Ok(());
    }
    // The schema describes the declared version only, so the carried
    // settings are checked once, resolved with their neighbour layer.
    declaration
        .resolve(
            &extension.name,
            effective[0].as_ref(),
            effective[1].as_ref(),
        )
        .map_err(|issues| {
            let first = issues
                .first()
                .map(|issue| format!("{} at {}", issue.code, issue.pointer))
                .unwrap_or_default();
            format!("the migrated settings don't satisfy the schema: {first}")
        })?;
    for (scope, before, values) in carried {
        let namespace = ConfigNamespace::new(extension.name.clone(), scope)
            .map_err(|_| "its name can't hold configuration".to_string())?;
        extension_config_store::compare_exchange_tx(
            conn,
            &namespace,
            Some(before.stamp()),
            declaration.version,
            &values,
        )
        .map_err(|error| match error {
            ConfigStoreError::Storage(storage) => storage.to_string(),
            _ => format!(
                "the migrated {} settings could not be saved",
                scope_word(scope)
            ),
        })?;
    }
    Ok(())
}

fn scope_word(scope: Scope) -> &'static str {
    match scope {
        Scope::Project => "project",
        Scope::Personal => "personal",
    }
}
