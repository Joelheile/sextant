use std::path::PathBuf;
use std::process::Command;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        build_language_shim();
    }
    tauri_build::build();
}

fn xcrun(args: &[&str]) -> String {
    let out = Command::new("xcrun")
        .args(args)
        .output()
        .expect("xcrun runs");
    assert!(out.status.success(), "xcrun {args:?} failed");
    String::from_utf8(out.stdout)
        .expect("utf-8")
        .trim()
        .to_string()
}

/// Compiles `swift/Language.swift` into a static library that links against Apple's
/// Foundation Models framework and the Swift runtime that ships with macOS.
fn build_language_shim() {
    const SOURCE: &str = "swift/Language.swift";
    println!("cargo:rerun-if-changed={SOURCE}");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let status = Command::new("xcrun")
        .args([
            "swiftc",
            "-emit-library",
            "-static",
            "-parse-as-library",
            "-module-name",
            "SextantLanguage",
            "-swift-version",
            "5",
            "-O",
            "-target",
            "arm64-apple-macosx26.0",
            SOURCE,
            "-o",
        ])
        .arg(out.join("libsextant_language.a"))
        .status()
        .expect("swiftc runs");
    assert!(status.success(), "swiftc failed on {SOURCE}");

    let toolchain = PathBuf::from(xcrun(&["-f", "swiftc"]));
    let toolchain_lib = toolchain
        .parent()
        .and_then(|bin| bin.parent())
        .expect("toolchain root")
        .join("lib/swift/macosx");
    let sdk = xcrun(&["--sdk", "macosx", "--show-sdk-path"]);
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-search=native={}", toolchain_lib.display());
    println!("cargo:rustc-link-search=native={sdk}/usr/lib/swift");
    println!("cargo:rustc-link-lib=static=sextant_language");
    println!("cargo:rustc-link-lib=framework=FoundationModels");
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}
