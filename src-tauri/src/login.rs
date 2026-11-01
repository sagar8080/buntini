//! Open at login through macOS's own login items (`SMAppService.mainAppService`).
//! Only an installed app with a real signature can register. Development and
//! ad-hoc builds report "unavailable" and never claim to be registered.
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LoginStatus {
    /// macOS will open Buntini at login.
    Enabled,
    /// Registered, but the person still has to allow it in System Settings.
    RequiresApproval,
    NotRegistered,
    /// macOS could not find the app's login item.
    NotFound,
    /// A development, unbundled or unsigned build: macOS cannot register it.
    Unavailable,
}

impl LoginStatus {
    /// What a saved "Open at login" setting can honestly say.
    pub fn registered(self) -> bool {
        matches!(self, Self::Enabled | Self::RequiresApproval)
    }
    fn from_raw(raw: isize) -> Self {
        match raw {
            0 => Self::NotRegistered,
            1 => Self::Enabled,
            2 => Self::RequiresApproval,
            3 => Self::NotFound,
            _ => Self::Unavailable,
        }
    }
}

const UNAVAILABLE: &str =
    "Open at login works in the installed, signed Buntini app, not in a development build.";

/// A login item points at the installed bundle. A binary outside an .app, as
/// `cargo test` and `tauri dev` run, has nothing macOS could open at login.
fn in_bundle(exe: &std::path::Path) -> bool {
    exe.to_string_lossy().contains(".app/Contents/MacOS/")
}
#[cfg(test)]
fn eligible(exe: &std::path::Path, signed: bool) -> bool {
    signed && in_bundle(exe)
}

#[cfg(target_os = "macos")]
mod native {
    use super::LoginStatus;
    use objc2::{msg_send, rc::Retained, runtime::AnyClass, runtime::AnyObject};
    use objc2_foundation::NSError;

    #[link(name = "ServiceManagement", kind = "framework")]
    extern "C" {}

    /// Ad-hoc and unsigned builds fail Apple's chain; Developer ID and Apple
    /// Development signatures pass.
    pub fn signed() -> bool {
        use security_framework::os::macos::code_signing::{Flags, SecCode, SecRequirement};
        let Ok(requirement) = "anchor apple generic".parse::<SecRequirement>() else {
            return false;
        };
        SecCode::for_self(Flags::NONE)
            .and_then(|code| code.check_validity(Flags::NONE, &requirement))
            .is_ok()
    }

    fn service() -> Option<Retained<AnyObject>> {
        let class = AnyClass::get(c"SMAppService")?;
        unsafe { msg_send![class, mainAppService] }
    }

    pub fn status() -> LoginStatus {
        let Some(service) = service() else {
            return LoginStatus::Unavailable;
        };
        let raw: isize = unsafe { msg_send![&*service, status] };
        LoginStatus::from_raw(raw)
    }

    pub fn set(enabled: bool) -> Result<(), String> {
        let service = service().ok_or(super::UNAVAILABLE)?;
        let result: Result<(), Retained<NSError>> = unsafe {
            if enabled {
                msg_send![&*service, registerAndReturnError: _]
            } else {
                msg_send![&*service, unregisterAndReturnError: _]
            }
        };
        result.map_err(|error| error.localizedDescription().to_string())
    }
}

fn available() -> bool {
    #[cfg(target_os = "macos")]
    {
        std::env::current_exe().is_ok_and(|exe| in_bundle(&exe) && native::signed())
    }
    #[cfg(not(target_os = "macos"))]
    false
}

/// What macOS reports right now, which can differ from the saved setting when
/// the person changed it in System Settings → General → Login Items.
pub fn status() -> LoginStatus {
    if !available() {
        return LoginStatus::Unavailable;
    }
    #[cfg(target_os = "macos")]
    return native::status();
    #[cfg(not(target_os = "macos"))]
    LoginStatus::Unavailable
}

/// Asks macOS to add or remove the login item, then reads the status back as
/// the receipt. Success is only what macOS confirms.
pub fn apply(enabled: bool) -> Result<LoginStatus, String> {
    if !available() {
        return Err(UNAVAILABLE.into());
    }
    #[cfg(target_os = "macos")]
    {
        let before = native::status();
        if before.registered() != enabled {
            native::set(enabled)?;
        }
        let after = native::status();
        if after.registered() != enabled {
            return Err(if enabled {
                "macOS did not add Buntini to your login items.".into()
            } else {
                "macOS still lists Buntini in your login items. Remove it in System Settings → General → Login Items.".into()
            });
        }
        Ok(after)
    }
    #[cfg(not(target_os = "macos"))]
    Err(UNAVAILABLE.into())
}

/// Applies a changed "Open at login" setting and records the outcome. Returns
/// the value to save, so the stored setting never claims more than macOS did.
pub fn apply_setting(store: &crate::store::Store, enabled: bool) -> bool {
    match apply(enabled) {
        Ok(status) => {
            let detail = match status {
                LoginStatus::RequiresApproval => {
                    "Registered; waiting for your OK in System Settings → General → Login Items"
                }
                _ if enabled => "macOS will open Buntini at login",
                _ => "Removed from login items",
            };
            let _ = store.audit(
                if enabled {
                    "login_item.enabled"
                } else {
                    "login_item.disabled"
                },
                detail,
            );
            status.registered()
        }
        // A build that can't register has no login item to remove, so off is the truth.
        Err(error) if error == UNAVAILABLE && !enabled => {
            let _ = store.audit("login_item.disabled", "Nothing to remove in this build");
            false
        }
        Err(error) => {
            let _ = store.audit("login_item.failed", &error);
            // Turning it off failed: it is still on. Turning it on failed: it is off.
            !enabled
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unbundled_build_reports_unavailable() {
        // `cargo test` runs a bare binary in target/, like a development build.
        assert_eq!(status(), LoginStatus::Unavailable);
        assert!(apply(true).is_err());
        assert!(apply(false).is_err());
        let store = crate::store::Store::open(std::path::Path::new(":memory:")).unwrap();
        assert!(
            !apply_setting(&store, true),
            "a failed register must not be saved as on"
        );
        assert!(store
            .audit_entries()
            .unwrap()
            .iter()
            .any(|entry| entry.action == "login_item.failed"));
        // A setting saved as on by the installed app can always be turned off here.
        assert!(!apply_setting(&store, false), "nothing is registered in this build");
    }

    #[test]
    fn only_a_signed_installed_bundle_is_eligible() {
        let installed = std::path::Path::new("/Applications/Buntini.app/Contents/MacOS/buntini");
        assert!(eligible(installed, true));
        assert!(
            !eligible(installed, false),
            "ad-hoc or unsigned builds cannot register"
        );
        assert!(!eligible(
            std::path::Path::new("/tmp/target/debug/buntini"),
            true
        ));
    }

    #[test]
    fn statuses_match_service_management_values() {
        assert_eq!(LoginStatus::from_raw(0), LoginStatus::NotRegistered);
        assert_eq!(LoginStatus::from_raw(1), LoginStatus::Enabled);
        assert_eq!(LoginStatus::from_raw(2), LoginStatus::RequiresApproval);
        assert_eq!(LoginStatus::from_raw(3), LoginStatus::NotFound);
        assert!(LoginStatus::RequiresApproval.registered());
        assert!(!LoginStatus::NotFound.registered());
        assert_eq!(
            serde_json::to_value(LoginStatus::RequiresApproval).unwrap(),
            "requiresApproval"
        );
    }
}
