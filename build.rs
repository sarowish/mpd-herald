use std::{env, process::Command};

const BUILD_VERSION_ENV: &str = "MPD_HERALD_BUILD_VERSION";
const NIX_REVISION_ENV: &str = "MPD_HERALD_GIT_REV";

fn main() {
    println!("cargo:rerun-if-env-changed={NIX_REVISION_ENV}");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=src");

    if let Some(git_dir) = git_output(&["rev-parse", "--git-dir"]) {
        for path in ["HEAD", "index", "packed-refs", "refs"] {
            println!("cargo:rerun-if-changed={git_dir}/{path}");
        }
    }

    let package_version =
        env::var("CARGO_PKG_VERSION").expect("Cargo should set its package version");
    let build_version = env::var(NIX_REVISION_ENV)
        .ok()
        .filter(|revision| !revision.trim().is_empty())
        .map(|revision| format!("git-{}", revision.trim()))
        .or_else(|| git_build_version(&package_version))
        .unwrap_or(package_version);

    println!("cargo:rustc-env={BUILD_VERSION_ENV}={build_version}");
}

fn git_build_version(package_version: &str) -> Option<String> {
    let revision = git_output(&["rev-parse", "--short=8", "HEAD"])?;
    let dirty = git_output(&["status", "--porcelain", "--untracked-files=normal"])
        .is_some_and(|status| !status.is_empty());
    let release_tag = format!("v{package_version}");
    let tagged_release = !dirty
        && git_output(&[
            "describe",
            "--tags",
            "--exact-match",
            "--match",
            &release_tag,
            "HEAD",
        ])
        .is_some_and(|tag| tag == release_tag);

    if tagged_release {
        Some(package_version.to_owned())
    } else if dirty {
        Some(format!("git-{revision}-dirty"))
    } else {
        Some(format!("git-{revision}"))
    }
}

fn git_output(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8(output.stdout)
        .ok()
        .map(|output| output.trim().to_owned())
}
