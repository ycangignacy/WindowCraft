// Made by ycangignacy
//! Minimal WMI client for volume encryption and adapter bindings.

use crate::error::{EngineError, Result};
use crate::win::ComApartment;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, CoSetProxyBlanket, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL,
    RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::System::Wmi::*;
use windows::core::{BSTR, PCWSTR, w};

fn connect(namespace: &str) -> Result<IWbemServices> {
    let fail = |error: windows::core::Error| EngineError::windows("IWbemLocator::ConnectServer", namespace, &error);
    // SAFETY: The CLSID is valid and the caller entered an apartment.
    let locator: IWbemLocator = unsafe { CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER) }.map_err(fail)?;
    let empty = BSTR::new();
    // SAFETY: BSTRs outlive the call and empty credentials mean the caller's.
    let services = unsafe {
        locator.ConnectServer(&BSTR::from(namespace), &empty, &empty, &empty, 0, &empty, None)
    }
    .map_err(fail)?;
    // SAFETY: `services` is a live proxy, and this sets standard client security.
    unsafe {
        CoSetProxyBlanket(
            &services,
            RPC_C_AUTHN_WINNT,
            RPC_C_AUTHZ_NONE,
            PCWSTR::null(),
            RPC_C_AUTHN_LEVEL_CALL,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
        )
    }
    .map_err(fail)?;
    Ok(services)
}

/// Runs a parameterless method on every object a query returns. Returns how
/// many objects it ran on.
pub fn invoke_all(namespace: &str, query: &str, method: &str) -> Result<usize> {
    let _apartment = ComApartment::enter()?;
    let services = connect(namespace)?;
    let fail = |error: windows::core::Error| EngineError::windows("IWbemServices", query, &error);

    // SAFETY: BSTRs outlive the call and the flags are valid.
    let results = unsafe {
        services.ExecQuery(
            &BSTR::from("WQL"),
            &BSTR::from(query),
            WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
            None,
        )
    }
    .map_err(fail)?;

    let mut count = 0;
    loop {
        let mut row = [None];
        let mut returned = 0;
        // SAFETY: `row` has room for one object and `returned` is an out pointer.
        unsafe { results.Next(WBEM_INFINITE, &mut row, &mut returned) }.ok().map_err(fail)?;
        let [Some(object)] = row else { break };
        if returned == 0 {
            break;
        }

        let mut path = VARIANT::default();
        // SAFETY: `object` is live, `__PATH` is a system property, and `path`
        // is a valid out pointer that VARIANT's Drop clears.
        unsafe { object.Get(w!("__PATH"), 0, &mut path, None, None) }.map_err(fail)?;
        // SAFETY: `__PATH` is always a VT_BSTR, so this union member is active.
        let path = unsafe { BSTR::clone(&path.Anonymous.Anonymous.Anonymous.bstrVal) };
        // SAFETY: The method takes no input parameters or output object here.
        unsafe {
            services.ExecMethod(&path, &BSTR::from(method), WBEM_GENERIC_FLAG_TYPE(0), None, None, None, None)
        }
        .map_err(fail)?;
        count += 1;
    }
    Ok(count)
}

/// Starts BitLocker decryption of the system drive.
pub fn decrypt_system_volume() -> Result<()> {
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_owned());
    let query = format!("SELECT * FROM Win32_EncryptableVolume WHERE DriveLetter='{drive}'");
    invoke_all(r"ROOT\CIMV2\Security\MicrosoftVolumeEncryption", &query, "Decrypt").map(drop)
}

/// Binds or unbinds the IPv6 stack on all adapters.
pub fn set_ipv6_binding(enabled: bool) -> Result<()> {
    let query = "SELECT * FROM MSFT_NetAdapterBindingSettingData WHERE ComponentID='ms_tcpip6'";
    let method = if enabled { "Enable" } else { "Disable" };
    invoke_all(r"ROOT\StandardCimv2", query, method).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_with_no_matches_invokes_nothing() {
        let count = invoke_all(
            r"ROOT\CIMV2",
            "SELECT * FROM Win32_Process WHERE Name='windowcraft-no-such-process.exe'",
            "Terminate",
        )
        .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn invoke_all_reports_errors_for_invalid_namespace_query_and_class() {
        let bad_ns = invoke_all(r"ROOT\WindowCraft_NoSuchNamespace", "SELECT * FROM Win32_Process", "NoOp").unwrap_err();
        assert!(matches!(
            &bad_ns,
            EngineError::Win32 { operation: "IWbemLocator::ConnectServer", target, .. }
                if target == r"ROOT\WindowCraft_NoSuchNamespace"
        ));

        let bad_wql = invoke_all(r"ROOT\CIMV2", "NOT A VALID WQL QUERY", "NoOp").unwrap_err();
        assert!(matches!(
            &bad_wql,
            EngineError::Win32 { operation: "IWbemServices", target, .. }
                if target == "NOT A VALID WQL QUERY"
        ));

        let bad_class = invoke_all(r"ROOT\CIMV2", "SELECT * FROM WindowCraft_NoSuchClass", "NoOp").unwrap_err();
        assert!(matches!(
            &bad_class,
            EngineError::Win32 { operation: "IWbemServices", target, .. }
                if target == "SELECT * FROM WindowCraft_NoSuchClass"
        ));
    }
}
