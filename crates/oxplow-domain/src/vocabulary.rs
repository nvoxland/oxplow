//! The vocabulary (P8.D1, `.context/data-model.md` "event_log",
//! `.context/extensions.md`):
//! what the event log accepts — every event type at every version, with
//! its schema and upcasts — and what a ref may name — every ref kind with
//! its id shape. Core's, plus what the running extensions declare
//! (`event_types:`, `ref_kinds`), so it changes while the app runs: a
//! [`VocabularyHandle`] holds the current [`Vocabulary`] and swaps it
//! whole when the extension catalog changes. A writer takes one snapshot
//! for a transaction (`current()`), so a swap never lands mid-way through
//! one; two handles never share kinds.

use std::sync::{Arc, RwLock};

use serde_json::Value;

use crate::events::schema::EventSchemaRegistry;
use crate::events::Envelope;
use crate::refs::kind::{core_kinds, KindRegistry};
use crate::DomainError;

/// The event types and ref kinds one process speaks, as of one swap.
pub struct Vocabulary {
    pub events: EventSchemaRegistry,
    pub kinds: KindRegistry,
}

impl Vocabulary {
    pub fn new(events: EventSchemaRegistry, kinds: KindRegistry) -> Self {
        Self { events, kinds }
    }

    /// Core's event types and ref kinds, nothing more.
    pub fn core() -> Self {
        Self::new(EventSchemaRegistry::core(), core_kinds())
    }

    /// Refuse a payload that isn't valid for a registered `type@v`.
    pub fn validate(&self, event_type: &str, v: u32, payload: &Value) -> Result<(), DomainError> {
        self.events.validate(event_type, v, payload)
    }

    /// An envelope's `type@v` and payload against its schema, and every
    /// subject as a canonical ref of a kind this vocabulary has (a
    /// consumer resolves subjects; a malformed one would dead-letter far
    /// from the producer that wrote it).
    pub fn validate_envelope(&self, env: &Envelope) -> Result<(), DomainError> {
        self.events.validate(&env.event_type, env.v, &env.payload)?;
        for subject in &env.subject {
            crate::refs::validate_ref(&self.kinds, subject)
                .map_err(|e| DomainError::Invalid(format!("{} subject: {e}", env.event_type)))?;
        }
        Ok(())
    }

    /// Carry a payload written at `from_v` to the newest version of its
    /// type, validated there.
    pub fn upcast_to_latest(
        &self,
        event_type: &str,
        from_v: u32,
        payload: Value,
    ) -> Result<(u32, Value), DomainError> {
        self.events.upcast_to_latest(event_type, from_v, payload)
    }

    pub fn is_registered(&self, event_type: &str, v: u32) -> bool {
        self.events.is_registered(event_type, v)
    }

    pub fn latest(&self, event_type: &str) -> Option<u32> {
        self.events.latest(event_type)
    }

    pub fn schema(&self, event_type: &str, v: u32) -> Option<&Value> {
        self.events.schema(event_type, v)
    }

    pub fn owner(&self, event_type: &str, v: u32) -> Option<Option<&str>> {
        self.events.owner(event_type, v)
    }

    pub fn versions(&self) -> Vec<(String, u32)> {
        self.events.versions()
    }
}

/// The current vocabulary, swapped whole. Cloning shares it.
#[derive(Clone)]
pub struct VocabularyHandle(Arc<RwLock<Arc<Vocabulary>>>);

impl VocabularyHandle {
    pub fn new(vocabulary: Vocabulary) -> Self {
        Self(Arc::new(RwLock::new(Arc::new(vocabulary))))
    }

    /// A handle on core's vocabulary alone. Core's is the same for the
    /// whole process, so it is built once (compiling every core event
    /// type's schema) and shared; a swap replaces this handle's, never it.
    pub fn core() -> Self {
        static CORE: std::sync::LazyLock<Arc<Vocabulary>> =
            std::sync::LazyLock::new(|| Arc::new(Vocabulary::core()));
        Self(Arc::new(RwLock::new(CORE.clone())))
    }

    /// The vocabulary as it is now — what one transaction validates
    /// against from start to end.
    pub fn current(&self) -> Arc<Vocabulary> {
        self.0.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Replace it whole: what's already taken (a transaction under way)
    /// keeps the one it took.
    pub fn swap(&self, vocabulary: Vocabulary) {
        *self.0.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(vocabulary);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::schema::{EventType, WorkItemTransitioned};
    use crate::refs::kind::KindSpec;

    /// Core's vocabulary is built once per process: compiling every core
    /// event type's schema was most of what building a test's services
    /// cost, and each store took its own handle. Handles still swap alone.
    #[test]
    fn core_handles_share_one_vocabulary_and_swap_alone() {
        let (a, b) = (VocabularyHandle::core(), VocabularyHandle::core());
        assert!(Arc::ptr_eq(&a.current(), &b.current()));
        a.swap(Vocabulary::new(EventSchemaRegistry::new(), core_kinds()));
        assert!(!Arc::ptr_eq(&a.current(), &b.current()));
        assert!(!b.current().versions().is_empty(), "b keeps core's");
    }

    /// What a writer holds sees a type swapped in after it was built; the
    /// snapshot it already took doesn't change under it.
    #[test]
    fn a_swap_reaches_the_holder_but_not_a_snapshot_taken() {
        let handle =
            VocabularyHandle::new(Vocabulary::new(EventSchemaRegistry::new(), core_kinds()));
        let held = handle.clone();
        let before = held.current();
        assert!(!before.is_registered(WorkItemTransitioned::TYPE, WorkItemTransitioned::V));
        handle.swap(Vocabulary::core());
        assert!(held
            .current()
            .is_registered(WorkItemTransitioned::TYPE, WorkItemTransitioned::V));
        assert!(!before.is_registered(WorkItemTransitioned::TYPE, WorkItemTransitioned::V));
    }

    /// Two handles in one process don't share kinds: what one adds, the
    /// other doesn't know.
    #[test]
    fn two_handles_dont_leak_kinds() {
        let (a, b) = (VocabularyHandle::core(), VocabularyHandle::core());
        let mut kinds = core_kinds();
        kinds
            .register(KindSpec::new("acme_pr", r"^\d+$").unwrap())
            .unwrap();
        a.swap(Vocabulary::new(EventSchemaRegistry::core(), kinds));
        assert!(crate::refs::validate_ref(&a.current().kinds, "acme_pr:12").is_ok());
        assert!(crate::refs::validate_ref(&b.current().kinds, "acme_pr:12").is_err());
    }
}
