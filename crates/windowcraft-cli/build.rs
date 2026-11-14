// Made by Nordan
//! Same DLL planting protection as the main binary: static imports resolve
//! from System32 only.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
    }
}
