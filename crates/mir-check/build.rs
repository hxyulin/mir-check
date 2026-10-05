#![forbid(unsafe_code)]

use std::process::Command;

const COMPILER_COMMIT: &str = "1303417c416e1595173d9689e7394c31e136ae95";

fn main() {
    println!("cargo:rerun-if-env-changed=RUSTC");
    let rustc = std::env::var("RUSTC").expect("Cargo supplies RUSTC");
    let version = output(&rustc, &["-vV"]);
    assert!(
        version.contains(COMPILER_COMMIT),
        "mir-check requires nightly-2026-09-22; compiler APIs and ABI must match"
    );
    let sysroot = output(&rustc, &["--print", "sysroot"]);
    println!("cargo:rustc-env=MIR_CHECK_SYSROOT={sysroot}");
    println!(
        "cargo:rustc-env=MIR_CHECK_COMPILER={}",
        version.lines().next().unwrap()
    );
    let target = std::env::var("CARGO_CFG_TARGET_OS").expect("Cargo supplies the target OS");
    if target == "linux" || target == "macos" {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{sysroot}/lib");
    }
}

fn output(program: &str, args: &[&str]) -> String {
    let result = Command::new(program)
        .args(args)
        .output()
        .expect("run the build compiler");
    assert!(result.status.success(), "build compiler invocation failed");
    String::from_utf8(result.stdout)
        .expect("compiler output is UTF-8")
        .trim()
        .to_owned()
}
