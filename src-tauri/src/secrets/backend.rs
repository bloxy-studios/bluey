//! The seam between [`super::SecretsStore`] and the macOS Keychain.
//!
//! Every operation names what it costs in the legacy (file-based) login
//! keychain, where an item trusts the code identity that created it
//! (docs/adr/0011-secret-storage-and-code-identity.md):
//! * [`SecretBackend::read`] decrypts the item — the only operation that can
//!   show the "Bluey wants to use your confidential information" prompt;
//! * [`SecretBackend::exists`], [`SecretBackend::list`] and
//!   [`SecretBackend::remove`] match attributes only and never prompt;
//! * [`SecretBackend::add`] creates a fresh item owned by the running build.
//!   There is deliberately no in-place modify: a modify by a build the item
//!   does not trust locks the item for every build, so a replace is always
//!   `remove` + `add`.
//!
//! Values travel as [`Zeroizing`] strings and never appear in errors or logs.

use bluey_core::error::RecoveryAction;
use bluey_core::BlueyError;
use zeroize::Zeroizing;

/// `errSecItemNotFound`.
pub const ERR_ITEM_NOT_FOUND: i32 = -25300;
/// `errSecDuplicateItem`.
pub const ERR_DUPLICATE_ITEM: i32 = -25299;
/// `errSecAuthFailed`: the password prompt was refused, or (with user
/// interaction disabled) this build is not trusted to decrypt the item.
pub const ERR_AUTH_FAILED: i32 = -25293;
/// `errSecUserCanceled`: the prompt was cancelled.
pub const ERR_USER_CANCELED: i32 = -128;
/// `errSecInteractionNotAllowed`: a prompt was needed while user interaction
/// was disabled (the credential-health probe, a locked keychain).
pub const ERR_INTERACTION_NOT_ALLOWED: i32 = -25308;
/// `errSecNotAvailable`, `errSecNoSuchKeychain`, `errSecInvalidKeychain`,
/// `errSecNoDefaultKeychain`: there is no usable login keychain.
const ERR_UNAVAILABLE: [i32; 4] = [-25291, -25294, -25295, -25307];

/// A failed Keychain call, by `OSStatus` (never `errSecSuccess`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeychainStatus(pub i32);

/// What a failure means for the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeychainFailure {
    /// macOS refused access: the prompt was denied or cancelled, or this build
    /// is not (yet) trusted for the item.
    Denied,
    /// A prompt would have been needed but user interaction is disabled.
    InteractionNotAllowed,
    /// No usable login keychain.
    Unavailable,
    /// Anything else.
    Other,
}

impl KeychainStatus {
    pub fn failure(self) -> KeychainFailure {
        match self.0 {
            ERR_AUTH_FAILED | ERR_USER_CANCELED => KeychainFailure::Denied,
            ERR_INTERACTION_NOT_ALLOWED => KeychainFailure::InteractionNotAllowed,
            status if ERR_UNAVAILABLE.contains(&status) => KeychainFailure::Unavailable,
            _ => KeychainFailure::Other,
        }
    }

    /// Whether the item exists but this build may not read it right now — the
    /// "locked" credential state (as opposed to a missing one).
    pub fn is_locked(self) -> bool {
        matches!(
            self.failure(),
            KeychainFailure::Denied | KeychainFailure::InteractionNotAllowed
        )
    }

    /// The user-facing error of a failed `op` on a key of `category`
    /// ([`super::key_category`]). Codes are stable (`src/lib/errors/present.ts`);
    /// `details` carry the operation, the category and the `OSStatus`, never a
    /// key id beyond its category and never a value.
    pub fn into_error(self, op: &str, category: &str) -> BlueyError {
        let details = serde_json::json!({ "op": op, "category": category, "status": self.0 });
        let error = match self.failure() {
            KeychainFailure::Denied => BlueyError::storage(
                "keychain_access_denied",
                "macOS blocked Bluey from using a saved credential — choose Always Allow when macOS asks",
            )
            .recoverable(RecoveryAction::Retry),
            KeychainFailure::InteractionNotAllowed => BlueyError::storage(
                "keychain_interaction_not_allowed",
                "macOS needs your approval before Bluey can use a saved credential",
            )
            .recoverable(RecoveryAction::Retry),
            KeychainFailure::Unavailable => BlueyError::storage(
                "keychain_unavailable",
                "the login keychain is not available",
            ),
            KeychainFailure::Other => {
                let message = match op {
                    "read" => "failed to read from the keychain",
                    "write" => "failed to write to the keychain",
                    "delete" => "failed to delete from the keychain",
                    _ => "the keychain request failed",
                };
                BlueyError::storage("keychain", message)
            }
        };
        error.with_details(details)
    }
}

/// Storage of the secrets of one Keychain service. Implementations are
/// synchronous and may block (a prompt waits for the user): callers run them
/// off the async runtime.
pub trait SecretBackend: Send + Sync {
    /// Decrypt `account`'s value (`None` when there is no item). May prompt.
    fn read(&self, account: &str) -> Result<Option<Zeroizing<String>>, KeychainStatus>;
    /// Whether an item exists — attributes only, never prompts.
    fn exists(&self, account: &str) -> Result<bool, KeychainStatus>;
    /// Create an item owned by the running build; fails with
    /// `errSecDuplicateItem` when one exists (callers `remove` first).
    fn add(&self, account: &str, value: &str) -> Result<(), KeychainStatus>;
    /// Delete the item — found by attributes, status checked, never prompts.
    /// `Ok(false)` when there was nothing to delete.
    fn remove(&self, account: &str) -> Result<bool, KeychainStatus>;
    /// The accounts of every item of the service — attributes only.
    fn list(&self) -> Result<Vec<String>, KeychainStatus>;
    /// [`Self::read`] with user interaction disabled: whether this build can
    /// read the item silently. A would-be prompt fails with a locked status
    /// instead of showing.
    fn probe(&self, account: &str) -> Result<Option<Zeroizing<String>>, KeychainStatus>;
}

#[cfg(test)]
pub mod fake {
    //! [`CountingFake`]: an in-memory Keychain that records every operation, so
    //! tests can pin how many prompting reads and writes an action costs.

    use std::collections::{HashMap, HashSet};

    use parking_lot::Mutex;
    use zeroize::Zeroizing;

    use super::{KeychainStatus, SecretBackend, ERR_DUPLICATE_ITEM};

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum Op {
        Read,
        Exists,
        Add,
        Remove,
        List,
        Probe,
    }

    #[derive(Default)]
    struct Inner {
        items: HashMap<String, String>,
        ops: Vec<(Op, String)>,
        /// Accounts whose data reads fail with this status (a build the item
        /// does not trust, a denied prompt).
        locked: HashMap<String, i32>,
        /// Accounts whose removal fails with this status.
        failing_removes: HashMap<String, i32>,
        /// Attribute-only lookups (`exists`, `list`) fail with this status.
        fail_lookups: Option<i32>,
    }

    /// Faithful to the Keychain where it matters: `add` refuses duplicates,
    /// `remove` reports whether something was deleted, a locked item still
    /// shows up in attribute-only lookups.
    #[derive(Default)]
    pub struct CountingFake {
        inner: Mutex<Inner>,
    }

    impl CountingFake {
        pub fn with_items(items: &[(&str, &str)]) -> Self {
            let fake = Self::default();
            fake.inner.lock().items = items
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            fake
        }

        /// Data reads of `account` fail with `status` until [`Self::unlock`].
        pub fn lock_item(&self, account: &str, status: i32) {
            self.inner.lock().locked.insert(account.into(), status);
        }

        pub fn unlock(&self, account: &str) {
            self.inner.lock().locked.remove(account);
        }

        pub fn fail_remove(&self, account: &str, status: i32) {
            self.inner
                .lock()
                .failing_removes
                .insert(account.into(), status);
        }

        pub fn fail_lookups(&self, status: i32) {
            self.inner.lock().fail_lookups = Some(status);
        }

        pub fn value(&self, account: &str) -> Option<String> {
            self.inner.lock().items.get(account).cloned()
        }

        pub fn accounts(&self) -> HashSet<String> {
            self.inner.lock().items.keys().cloned().collect()
        }

        /// How many `op`s ran (on any account).
        pub fn count(&self, op: Op) -> usize {
            self.inner
                .lock()
                .ops
                .iter()
                .filter(|(o, _)| *o == op)
                .count()
        }

        /// How many `op`s ran on `account`.
        pub fn count_for(&self, op: Op, account: &str) -> usize {
            self.inner
                .lock()
                .ops
                .iter()
                .filter(|(o, a)| *o == op && a == account)
                .count()
        }

        /// Prompting operations: data reads (probes never prompt).
        pub fn reads(&self) -> usize {
            self.count(Op::Read)
        }

        pub fn writes(&self) -> usize {
            self.count(Op::Add)
        }

        pub fn reset_counts(&self) {
            self.inner.lock().ops.clear();
        }

        fn note(&self, op: Op, account: &str) {
            self.inner.lock().ops.push((op, account.to_string()));
        }
    }

    impl SecretBackend for CountingFake {
        fn read(&self, account: &str) -> Result<Option<Zeroizing<String>>, KeychainStatus> {
            self.note(Op::Read, account);
            let inner = self.inner.lock();
            if let Some(status) = inner.locked.get(account) {
                return Err(KeychainStatus(*status));
            }
            Ok(inner.items.get(account).cloned().map(Zeroizing::new))
        }

        fn exists(&self, account: &str) -> Result<bool, KeychainStatus> {
            self.note(Op::Exists, account);
            let inner = self.inner.lock();
            if let Some(status) = inner.fail_lookups {
                return Err(KeychainStatus(status));
            }
            Ok(inner.items.contains_key(account))
        }

        fn add(&self, account: &str, value: &str) -> Result<(), KeychainStatus> {
            self.note(Op::Add, account);
            let mut inner = self.inner.lock();
            if inner.items.contains_key(account) {
                return Err(KeychainStatus(ERR_DUPLICATE_ITEM));
            }
            inner.items.insert(account.into(), value.into());
            // A fresh item is owned by the build that added it.
            inner.locked.remove(account);
            Ok(())
        }

        fn remove(&self, account: &str) -> Result<bool, KeychainStatus> {
            self.note(Op::Remove, account);
            let mut inner = self.inner.lock();
            if let Some(status) = inner.failing_removes.get(account) {
                return Err(KeychainStatus(*status));
            }
            Ok(inner.items.remove(account).is_some())
        }

        fn list(&self) -> Result<Vec<String>, KeychainStatus> {
            self.note(Op::List, "*");
            let inner = self.inner.lock();
            if let Some(status) = inner.fail_lookups {
                return Err(KeychainStatus(status));
            }
            Ok(inner.items.keys().cloned().collect())
        }

        fn probe(&self, account: &str) -> Result<Option<Zeroizing<String>>, KeychainStatus> {
            self.note(Op::Probe, account);
            let inner = self.inner.lock();
            if let Some(status) = inner.locked.get(account) {
                return Err(KeychainStatus(*status));
            }
            Ok(inner.items.get(account).cloned().map(Zeroizing::new))
        }
    }
}
