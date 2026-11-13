// Made by WindowCraft Dev Team
//! AppX and MSIX packages through WinRT, without `Get-AppxPackage`.

use crate::error::{EngineError, Result};
use crate::jobs::CancelToken;
use crate::registry::{RegPath, RegistryScope};
use crate::win::{ComApartment, wildcard_match};
use windows_collections::IIterable;
use windows::Management::Deployment::{DeploymentOptions, PackageManager, PackageVolume, RemovalOptions};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::{HSTRING, PWSTR};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPackage {
    pub name: String,
    pub full_name: String,
    pub family_name: String,
    pub version: String,
}

fn fail(operation: &'static str, target: &str, error: &windows::core::Error) -> EngineError {
    EngineError::windows(operation, target, error)
}

fn collect(iterable: impl IntoIterator<Item = windows::ApplicationModel::Package>) -> Vec<InstalledPackage> {
    let mut packages = Vec::new();
    for package in iterable {
        let Ok(id) = package.Id() else { continue };
        let (Ok(name), Ok(full_name), Ok(family_name), Ok(version)) =
            (id.Name(), id.FullName(), id.FamilyName(), id.Version())
        else {
            continue;
        };
        packages.push(InstalledPackage {
            name: name.to_string(),
            full_name: full_name.to_string(),
            family_name: family_name.to_string(),
            version: format!("{}.{}.{}.{}", version.Major, version.Minor, version.Build, version.Revision),
        });
    }
    packages
}

/// Packages for the current user. Works without elevation.
pub fn installed() -> Result<Vec<InstalledPackage>> {
    let _apartment = ComApartment::enter()?;
    let manager = PackageManager::new().map_err(|e| fail("PackageManager", "AppX", &e))?;
    let packages = manager
        .FindPackagesByUserSecurityId(&HSTRING::new())
        .map_err(|e| fail("FindPackagesByUserSecurityId", "AppX", &e))?;
    Ok(collect(packages))
}

/// Removes every matching package for all users and deprovisions it so new
/// accounts do not receive it. Returns the number of packages removed.
pub fn remove(pattern: &str, cancel: &CancelToken) -> Result<usize> {
    let _apartment = ComApartment::enter()?;
    let manager = PackageManager::new().map_err(|e| fail("PackageManager", "AppX", &e))?;
    let packages = manager.FindPackages().map_err(|e| fail("FindPackages", pattern, &e))?;
    let matching: Vec<InstalledPackage> =
        collect(packages).into_iter().filter(|package| wildcard_match(pattern, &package.name)).collect();

    let mut removed = 0;
    let mut families: Vec<String> = Vec::new();
    for package in &matching {
        cancel.check()?;
        let full_name = HSTRING::from(&package.full_name);
        let operation = manager
            .RemovePackageWithOptionsAsync(&full_name, RemovalOptions::RemoveForAllUsers)
            .map_err(|e| fail("RemovePackageAsync", &package.name, &e))?;
        operation.join().map_err(|e| fail("RemovePackageAsync", &package.name, &e))?;
        removed += 1;
        if !families.contains(&package.family_name) {
            families.push(package.family_name.clone());
        }
    }
    for family in families {
        cancel.check()?;
        let operation = manager
            .DeprovisionPackageForAllUsersAsync(&HSTRING::from(&family))
            .map_err(|e| fail("DeprovisionPackageForAllUsersAsync", &family, &e))?;
        // Packages that were never provisioned report an error here, which
        // does not affect the removal that already succeeded.
        let _ = operation.join();
    }
    Ok(removed)
}

/// Re-registers an installed package by family name.
pub fn register_family(family: &str) -> Result<()> {
    let _apartment = ComApartment::enter()?;
    let manager = PackageManager::new().map_err(|e| fail("PackageManager", family, &e))?;
    let operation = manager
        .RegisterPackageByFamilyNameAndOptionalPackagesAsync(
            &HSTRING::from(family),
            None::<&IIterable<HSTRING>>,
            DeploymentOptions::ForceApplicationShutdown,
            None::<&PackageVolume>,
            None::<&IIterable<HSTRING>>,
        )
        .map_err(|e| fail("RegisterPackageByFamilyNameAsync", family, &e))?;
    operation.join().map_err(|e| fail("RegisterPackageByFamilyNameAsync", family, &e))?;
    Ok(())
}

/// Text SID of the current user.
pub fn current_user_sid() -> Result<String> {
    let mut token = HANDLE::default();
    // SAFETY: The pseudo-handle is always valid and `token` is an out pointer.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
        .map_err(|e| fail("OpenProcessToken", "current process", &e))?;
    let token = crate::win::HandleGuard(token);

    let mut size = 0;
    // SAFETY: A null buffer asks for the size, so the expected error is ignored.
    let _ = unsafe { GetTokenInformation(token.raw(), TokenUser, None, 0, &mut size) };
    let mut storage = vec![0u64; (size as usize).div_ceil(8)];
    // SAFETY: `storage` provides `size` aligned, writable bytes.
    unsafe { GetTokenInformation(token.raw(), TokenUser, Some(storage.as_mut_ptr().cast()), size, &mut size) }
        .map_err(|e| fail("GetTokenInformation", "current process", &e))?;

    let user = storage.as_ptr().cast::<TOKEN_USER>();
    let mut text = PWSTR::null();
    // SAFETY: The buffer holds a TOKEN_USER, and the output string is
    // LocalAlloc'd and moved into a guard below.
    unsafe { ConvertSidToStringSidW((*user).User.Sid, &mut text) }
        .map_err(|e| fail("ConvertSidToStringSidW", "current user", &e))?;
    let _guard = crate::win::LocalGuard::pwstr(text);
    // SAFETY: On success `text` is a valid null-terminated string.
    Ok(unsafe { crate::win::from_pcwstr(windows::core::PCWSTR(text.0)) })
}

/// Registers a package under `EndOfLife`, which stops Windows reinstalling it.
pub fn mark_end_of_life(pattern: &str) -> Result<()> {
    let sid = current_user_sid()?;
    for package in installed()?.into_iter().filter(|package| wildcard_match(pattern, &package.name)) {
        let path = RegPath::parse(&format!(
            r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Appx\AppxAllUserStore\EndOfLife\{sid}\{}",
            package.full_name
        ))?;
        drop(RegistryScope::System.open_for_write(&path)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_packages_for_the_current_user() {
        let packages = installed().unwrap();
        assert!(!packages.is_empty());
        assert!(packages.iter().all(|package| !package.name.is_empty()));
        assert!(packages.iter().all(|package| !package.full_name.is_empty()));
        assert!(packages.iter().all(|package| !package.family_name.is_empty()));
        assert!(packages.iter().all(|package| package.full_name.starts_with(&package.name)));
        assert!(packages.iter().all(|package| {
            let parts: Vec<&str> = package.version.split('.').collect();
            parts.len() == 4 && parts.iter().all(|part| part.parse::<u16>().is_ok())
        }));
    }

    #[test]
    fn reads_the_current_user_sid() {
        assert!(current_user_sid().unwrap().starts_with("S-1-5-"));
    }

    #[test]
    fn collect_returns_empty_list_when_given_no_packages() {
        assert!(collect(std::iter::empty()).is_empty());
    }

    #[test]
    fn fail_maps_access_denied_to_needs_elevation_and_preserves_other_hresults() {
        let denied = windows::core::Error::from_hresult(windows::core::HRESULT(0x8007_0005_u32.cast_signed()));
        let err = fail("FindPackages", "AppX", &denied);
        assert!(matches!(&err, EngineError::NeedsElevation(detail) if detail == "FindPackages on AppX"));

        let missing = windows::core::Error::from_hresult(windows::core::HRESULT(0x8007_0490_u32.cast_signed()));
        let err = fail("RemovePackageAsync", "Clipchamp", &missing);
        assert!(err.is_not_found());
        assert!(matches!(
            &err,
            EngineError::Win32 { operation: "RemovePackageAsync", target, code: 0x8007_0490, .. }
                if target == "Clipchamp"
        ));
    }

    #[test]
    fn mark_end_of_life_with_unmatched_pattern_performs_no_registry_writes() {
        mark_end_of_life("WindowCraft.DefinitelyNotInstalled.98765").unwrap();
    }
}
