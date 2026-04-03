use std::process::Command;

fn main() {
    // Rebuild only if typescript files change
    println!("cargo:rerun-if-changed=ts");

    // Run webpack to compile typescript
    let status = Command::new("npx")
        .args(&["webpack", "--mode", "production"])
        .status()
        .expect("Failed to execute webpack");

    if !status.success() {
        panic!("Webpack build failed");
    }
}
