use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=tools/steam-query/Cargo.toml");
    println!("cargo:rerun-if-changed=tools/steam-query/Cargo.lock");
    println!("cargo:rerun-if-changed=tools/steam-query/src/main.rs");
    if !env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows") {
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let target = out.join("steam-target");
    let profile = env::var("PROFILE").expect("PROFILE");
    let mut command = Command::new(env::var_os("CARGO").expect("cargo"));
    command
        .arg("build")
        .arg("--locked")
        .arg("--manifest-path")
        .arg("tools/steam-query/Cargo.toml")
        .arg("--target-dir")
        .arg(&target);
    if profile == "release" {
        command.arg("--release");
    }
    let status = command
        .status()
        .expect("could not compile Steam query helper");
    assert!(status.success(), "Steam query helper failed to build");
    fs::copy(
        target.join(profile).join("gmm-steam-query.exe"),
        out.join("gmm-steam-query.exe"),
    )
    .expect("could not embed Steam query helper");
}
