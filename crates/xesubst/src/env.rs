//! Variable environment trait and standard implementations.

use std::collections::BTreeMap;

/// Trait abstracting variable storage and retrieval.
pub trait VariableTable {
    /// Gets the value of a variable if set.
    fn get(&self, name: &str) -> Option<&str>;

    /// Sets or updates a variable.
    fn set(&mut self, name: &str, value: &str);

    /// Checks if a variable is set (even if set to empty string).
    fn is_set(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Returns all variable names starting with `prefix`, sorted lexicographically.
    fn names_with_prefix(&self, prefix: &str) -> Vec<String>;

    /// Returns all variable names and values.
    fn all_vars(&self) -> Vec<(String, String)>;
}

/// An in-memory variable table backed by a BTreeMap.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EnvMap {
    vars: BTreeMap<String, String>,
}

impl EnvMap {
    /// Creates an empty environment map.
    pub fn new() -> Self {
        Self {
            vars: BTreeMap::new(),
        }
    }

    /// Populates from standard process environment.
    pub fn from_process_env() -> Self {
        let mut map = Self::new();
        for (k, v) in std::env::vars() {
            map.set(&k, &v);
        }
        map
    }
}

impl VariableTable for EnvMap {
    fn get(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(String::as_str)
    }

    fn set(&mut self, name: &str, value: &str) {
        self.vars.insert(name.to_string(), value.to_string());
    }

    fn names_with_prefix(&self, prefix: &str) -> Vec<String> {
        self.vars
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect()
    }

    fn all_vars(&self) -> Vec<(String, String)> {
        self.vars
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

/// Overlay environment that caches writes on top of an underlying immutable environment.
pub struct OverlayEnv<'a> {
    parent: &'a dyn VariableTable,
    overlay: BTreeMap<String, String>,
}

impl<'a> OverlayEnv<'a> {
    /// Creates a new overlay on top of `parent`.
    pub fn new(parent: &'a dyn VariableTable) -> Self {
        Self {
            parent,
            overlay: BTreeMap::new(),
        }
    }
}

impl<'a> VariableTable for OverlayEnv<'a> {
    fn get(&self, name: &str) -> Option<&str> {
        if let Some(val) = self.overlay.get(name) {
            Some(val.as_str())
        } else {
            self.parent.get(name)
        }
    }

    fn set(&mut self, name: &str, value: &str) {
        self.overlay.insert(name.to_string(), value.to_string());
    }

    fn names_with_prefix(&self, prefix: &str) -> Vec<String> {
        let mut set = std::collections::BTreeSet::new();
        for name in self.parent.names_with_prefix(prefix) {
            set.insert(name);
        }
        for name in self.overlay.keys() {
            if name.starts_with(prefix) {
                set.insert(name.clone());
            }
        }
        set.into_iter().collect()
    }

    fn all_vars(&self) -> Vec<(String, String)> {
        let mut map = BTreeMap::new();
        for (k, v) in self.parent.all_vars() {
            map.insert(k, v);
        }
        for (k, v) in &self.overlay {
            map.insert(k.clone(), v.clone());
        }
        map.into_iter().collect()
    }
}
