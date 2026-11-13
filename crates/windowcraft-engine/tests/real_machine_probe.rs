// Made by WindowCraft Dev Team
//! Read-only checks against the live system. They guard against fixtures that
//! drift from what Windows really stores. Nothing here writes anything.

use std::collections::BTreeMap;
use windowcraft_engine::autoruns::{self, EntrySource};
use windowcraft_engine::registry::RegistryScope;
use windowcraft_engine::{appx, cleaner, dns, dns_bench, gaming, packages, power, sysinfo, tasks};

#[test]
fn windows_binaries_in_autoruns_are_not_reported_as_unsigned() {
    let root = std::env::var("SystemRoot").unwrap().to_lowercase();
    let unsigned: Vec<String> = autoruns::scan()
        .into_iter()
        .filter(|entry| !entry.signed)
        .filter_map(|entry| entry.binary)
        .filter(|binary| binary.to_lowercase().starts_with(&root) && std::path::Path::new(binary).is_file())
        .collect();
    println!("unsigned under {root}: {unsigned:#?}");
    assert!(unsigned.is_empty(), "{} Windows binaries look unsigned", unsigned.len());
}

/// Real network traffic to every resolver in the catalog, so opt-in.
#[tokio::test]
#[ignore = "queries public DNS resolvers"]
async fn dns_benchmark_reports_each_protocol_honestly() {
    let catalog = windowcraft_config::Catalog::load().unwrap();
    let targets = catalog.dns.iter().filter_map(dns_bench::Target::from_provider).collect();
    let results = dns_bench::benchmark(targets, 3).await.unwrap();
    for result in &results {
        let protocols: Vec<String> = result
            .measurements
            .iter()
            .map(|m| format!("{}: {:.0} ms, {} failed", m.protocol.label(), m.average_ms, m.failures))
            .collect();
        println!("{}: score {:?} [{}]", result.id, result.score(), protocols.join(", "));
    }
    println!("fastest: {:?}", dns_bench::fastest(&results));
    assert!(dns_bench::fastest(&results).is_some());
    let broken: Vec<&str> = results
        .iter()
        .filter(|result| result.measurements.iter().any(|m| m.failures >= 3))
        .map(|result| result.id.as_str())
        .collect();
    assert!(broken.is_empty(), "every round failed for {broken:?}");
}

#[test]
fn scanners_return_real_data() {
    let devices = gaming::pci_devices(&RegistryScope::System).unwrap();
    let mut classes = BTreeMap::new();
    for device in &devices {
        *classes.entry(device.class.label()).or_insert(0) += 1;
    }
    println!("pci devices: {classes:?}");
    assert!(devices.iter().all(|device| !device.name.is_empty()), "every device needs a readable name");

    let entries = autoruns::scan();
    let count = |source| entries.iter().filter(|entry| entry.source == source).count();
    println!(
        "autoruns: run={} folder={} task={} service={} signed={}",
        count(EntrySource::RunKey),
        count(EntrySource::StartupFolder),
        count(EntrySource::Task),
        count(EntrySource::Service),
        entries.iter().filter(|entry| entry.signed).count(),
    );

    let programs = packages::installed_programs();
    println!("installed programs: {}", programs.len());
    assert!(!programs.is_empty());

    let adapters = dns::adapters().unwrap();
    println!("adapters: {} up={}", adapters.len(), adapters.iter().filter(|adapter| adapter.up).count());

    let packages = appx::installed().unwrap();
    println!("appx packages: {}", packages.len());
    assert!(!packages.is_empty());

    let schemes = power::schemes().unwrap();
    println!("power schemes: {:?}", schemes.iter().map(|scheme| scheme.name.as_str()).collect::<Vec<_>>());

    let timer = gaming::timer_resolution().unwrap();
    println!("timer: {timer:?}");

    let tasks = tasks::list_folder(r"\Microsoft\Windows\UpdateOrchestrator").unwrap();
    println!("update tasks: {}", tasks.len());

    let info = sysinfo::query().unwrap();
    println!("{} {} build {} ram {} GB", info.product, info.display_version, info.full_build(), info.ram_kb >> 20);

    for target in cleaner::targets() {
        let scan = cleaner::scan(&target);
        println!("cleaner {}: {} MB in {} files", target.id, scan.bytes >> 20, scan.files);
    }
}
