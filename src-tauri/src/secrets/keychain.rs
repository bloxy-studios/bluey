//! The production [`SecretBackend`]: generic passwords of one service in the
//! user's default (login) keychain — the same items `keyring` 3 created, so
//! saved keys keep working.
//!
//! It calls the legacy `SecKeychain*` API directly because the access pattern
//! decides whether macOS prompts (ADR 0011): lookups, enumeration and deletes
//! pass no data pointer (attributes only, never a prompt); only
//! [`SecretBackend::read`] decrypts; a replace is delete + add, never an
//! in-place modify.

use zeroize::Zeroizing;

use super::backend::{KeychainStatus, SecretBackend};

/// `errSecDecode`: the stored bytes are not UTF-8.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const ERR_DECODE: i32 = -26275;

/// Generic passwords of `service` in the login keychain.
pub struct KeychainBackend {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    service: String,
}

impl KeychainBackend {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }
}

/// Read another app's generic password (an explicit account import). Prompts
/// when macOS has not been told to Always Allow Bluey for that item.
pub fn read_foreign_item(
    service: &str,
    account: &str,
) -> Result<Option<Zeroizing<String>>, KeychainStatus> {
    KeychainBackend::new(service).read(account)
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ptr;

    use core_foundation::base::TCFType;
    use security_framework::item::{ItemClass, ItemSearchOptions, Limit};
    use security_framework::os::macos::keychain::{SecKeychain, SecPreferencesDomain};
    use security_framework::os::macos::keychain_item::SecKeychainItem;
    use security_framework::os::macos::passwords::find_generic_password;
    use security_framework_sys::keychain::SecKeychainFindGenericPassword;
    use security_framework_sys::keychain_item::SecKeychainItemDelete;
    use zeroize::{Zeroize, Zeroizing};

    use super::{KeychainBackend, ERR_DECODE};
    use crate::secrets::backend::{KeychainStatus, SecretBackend, ERR_ITEM_NOT_FOUND};

    fn status(error: security_framework::base::Error) -> KeychainStatus {
        KeychainStatus(error.code())
    }

    /// The user's default keychain — where `keyring` 3 stored Bluey's items.
    fn login_keychain() -> Result<SecKeychain, KeychainStatus> {
        SecKeychain::default_for_domain(SecPreferencesDomain::User).map_err(status)
    }

    impl KeychainBackend {
        /// The item without its data: `SecKeychainFindGenericPassword` with
        /// null password pointers, which never decrypts and never prompts.
        fn find_item(&self, account: &str) -> Result<Option<SecKeychainItem>, KeychainStatus> {
            let keychain = login_keychain()?;
            let mut item = ptr::null_mut();
            // SAFETY: the service / account pointers are valid for their
            // lengths for the duration of the call; `item` is an out-pointer
            // that receives a +1 reference on success.
            let code = unsafe {
                SecKeychainFindGenericPassword(
                    keychain.as_CFTypeRef(),
                    self.service.len() as u32,
                    self.service.as_ptr().cast(),
                    account.len() as u32,
                    account.as_ptr().cast(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    &mut item,
                )
            };
            match code {
                0 if !item.is_null() => {
                    // SAFETY: a non-null item from a successful find is owned (create rule).
                    Ok(Some(unsafe {
                        SecKeychainItem::wrap_under_create_rule(item)
                    }))
                }
                0 | ERR_ITEM_NOT_FOUND => Ok(None),
                code => Err(KeychainStatus(code)),
            }
        }
    }

    impl SecretBackend for KeychainBackend {
        fn read(&self, account: &str) -> Result<Option<Zeroizing<String>>, KeychainStatus> {
            let keychain = login_keychain()?;
            match find_generic_password(Some(&[keychain]), &self.service, account) {
                Ok((password, _item)) => match String::from_utf8(password.to_vec()) {
                    Ok(value) => Ok(Some(Zeroizing::new(value))),
                    Err(error) => {
                        error.into_bytes().zeroize();
                        Err(KeychainStatus(ERR_DECODE))
                    }
                },
                Err(error) if error.code() == ERR_ITEM_NOT_FOUND => Ok(None),
                Err(error) => Err(status(error)),
            }
        }

        fn exists(&self, account: &str) -> Result<bool, KeychainStatus> {
            Ok(self.find_item(account)?.is_some())
        }

        fn add(&self, account: &str, value: &str) -> Result<(), KeychainStatus> {
            login_keychain()?
                .add_generic_password(&self.service, account, value.as_bytes())
                .map_err(status)
        }

        fn remove(&self, account: &str) -> Result<bool, KeychainStatus> {
            let Some(item) = self.find_item(account)? else {
                return Ok(false);
            };
            // SAFETY: `item` is a live keychain item reference.
            match unsafe { SecKeychainItemDelete(item.as_concrete_TypeRef()) } {
                0 => Ok(true),
                ERR_ITEM_NOT_FOUND => Ok(false),
                code => Err(KeychainStatus(code)),
            }
        }

        fn list(&self) -> Result<Vec<String>, KeychainStatus> {
            let keychain = login_keychain()?;
            let results = ItemSearchOptions::new()
                .class(ItemClass::generic_password())
                .keychains(&[keychain])
                .service(&self.service)
                .load_attributes(true)
                .limit(Limit::All)
                .search();
            match results {
                Ok(results) => Ok(results
                    .iter()
                    .filter_map(|result| result.simplify_dict()?.remove("acct"))
                    .collect()),
                Err(error) if error.code() == ERR_ITEM_NOT_FOUND => Ok(Vec::new()),
                Err(error) => Err(status(error)),
            }
        }

        fn probe(&self, account: &str) -> Result<Option<Zeroizing<String>>, KeychainStatus> {
            // Process-wide until the guard drops; `SecretsStore` serializes
            // every backend call, so no other Bluey read is affected.
            let _no_prompts = SecKeychain::disable_user_interaction().map_err(status)?;
            self.read(account)
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    //! Bluey ships for macOS only; elsewhere (the Linux-host type check)
    //! there is no keychain: nothing is stored and writes fail.

    use zeroize::Zeroizing;

    use super::KeychainBackend;
    use crate::secrets::backend::{KeychainStatus, SecretBackend};

    /// `errSecNotAvailable`.
    const NOT_AVAILABLE: KeychainStatus = KeychainStatus(-25291);

    impl SecretBackend for KeychainBackend {
        fn read(&self, _: &str) -> Result<Option<Zeroizing<String>>, KeychainStatus> {
            Ok(None)
        }
        fn exists(&self, _: &str) -> Result<bool, KeychainStatus> {
            Ok(false)
        }
        fn add(&self, _: &str, _: &str) -> Result<(), KeychainStatus> {
            Err(NOT_AVAILABLE)
        }
        fn remove(&self, _: &str) -> Result<bool, KeychainStatus> {
            Ok(false)
        }
        fn list(&self) -> Result<Vec<String>, KeychainStatus> {
            Ok(Vec::new())
        }
        fn probe(&self, _: &str) -> Result<Option<Zeroizing<String>>, KeychainStatus> {
            Ok(None)
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    //! Opt-in (`cargo test --features dev-tools keychain_backend -- --ignored`):
    //! exercises the real login keychain under a throwaway service with user
    //! interaction disabled, so it can never show a dialog. Items this build
    //! creates are trusted by it, so no step needs approval.

    use security_framework::os::macos::keychain::SecKeychain;

    use super::*;

    #[test]
    #[ignore = "touches the login keychain; run explicitly on a Mac"]
    fn keychain_backend_round_trips_without_prompting() {
        let _no_prompts = SecKeychain::disable_user_interaction().expect("disable interaction");
        let service = format!(
            "com.codewithabdul.bluey.test-{}",
            uuid::Uuid::new_v4().simple()
        );
        let backend = KeychainBackend::new(&service);
        let account = "provider:test:api_key";

        assert_eq!(backend.exists(account), Ok(false));
        assert_eq!(backend.read(account).map(|v| v.is_none()), Ok(true));
        assert_eq!(backend.remove(account), Ok(false));

        backend.add(account, "first").expect("add");
        assert_eq!(backend.exists(account), Ok(true));
        assert_eq!(backend.list(), Ok(vec![account.to_string()]));
        assert_eq!(
            backend.add(account, "again"),
            Err(KeychainStatus(crate::secrets::backend::ERR_DUPLICATE_ITEM))
        );
        assert_eq!(backend.remove(account), Ok(true));
        backend.add(account, "second").expect("re-add");
        assert_eq!(
            backend
                .read(account)
                .expect("read")
                .as_deref()
                .map(String::as_str),
            Some("second")
        );
        // Last: the probe's own guard re-enables interaction when it drops.
        assert_eq!(
            backend
                .probe(account)
                .expect("probe")
                .as_deref()
                .map(String::as_str),
            Some("second")
        );
        assert_eq!(backend.remove(account), Ok(true));
        assert_eq!(backend.list(), Ok(Vec::new()));
    }
}
