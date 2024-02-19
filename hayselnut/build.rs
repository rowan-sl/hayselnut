use std::process::Command;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    embuild::espidf::sysenv::output();
    // some info on the build
    let commit_hash = Command::new("git")
        .args(["describe", "--always", "--dirty=-modified"])
        .output()
        .expect("failed to query git commit hash");
    assert!(
        commit_hash.status.success(),
        "failed to query git commit hash (`git describe` exit code not zero)"
    );
    println!(
        "cargo:rustc-env=BUILD_GIT_REV={}",
        String::from_utf8(commit_hash.stdout)
            .expect("`git describe` output not valid utf-8")
            .trim()
    );
    let now = chrono::Local::now();
    println!("cargo:rustc-env=BUILD_DATETIME_PRETTY={}", now.to_rfc2822());
    println!("cargo:rustc-env=BUILD_DATETIME={}", now.to_rfc3339());
    Ok(())
}
