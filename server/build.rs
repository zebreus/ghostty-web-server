//! Ensures `client/dist/` exists before `rust-embed` tries to read it.
//!
//! `trunk build` populates `client/dist/` with the real Yew bundle. When
//! someone runs `cargo run -p ghostty-web-server` (or `cargo check`) before
//! ever invoking trunk, the directory is missing and `#[derive(RustEmbed)]`
//! aborts compilation. We create an empty placeholder so the build always
//! succeeds; the served `/`, `/client.js`, `/client_bg.wasm` routes will
//! return 404 until trunk has run, which is the same UX as a stale build.

use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dist = manifest_dir.join("..").join("client").join("dist");
    if !dist.exists() {
        if let Err(e) = fs::create_dir_all(&dist) {
            // Don't fail the build — print a warning and let `rust-embed`
            // report a clearer error if the path is still unusable.
            println!(
                "cargo:warning=could not create {}: {e}",
                dist.display()
            );
        } else {
            // Drop a sentinel so the directory has at least one entry and
            // any tooling that looks for files doesn't trip on emptiness.
            let _ = fs::write(
                dist.join(".gitkeep"),
                "# placeholder created by server/build.rs; run `trunk build` in client/ to populate.\n",
            );
            println!(
                "cargo:warning=client/dist/ was missing; created an empty placeholder. \
                 Run `cd client && trunk build` to build the actual Yew bundle."
            );
        }
    }

    // Trigger a rebuild whenever the client bundle changes.
    println!("cargo:rerun-if-changed={}", dist.display());
    println!("cargo:rerun-if-changed=assets");
}
