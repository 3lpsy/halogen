//! Track frontend content in both Cargo's rebuild graph and sccache's compiler key.

use sha2::{Digest, Sha256};
use std::path::Path;

fn main() {
    if std::env::var_os("CARGO_FEATURE_EMBED_FRONTEND").is_none() {
        return;
    }
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist");
    println!("cargo:rerun-if-changed={}", dist.display());
    assert!(
        dist.join("index.html").is_file(),
        "embed-frontend requires `just ui-build` first"
    );
    let mut digest = Sha256::new();
    let mut dependencies = String::new();
    hash_dir(&dist, &dist, &mut digest, &mut dependencies);
    let output =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("build output directory"));
    std::fs::write(output.join("frontend_dependencies.rs"), dependencies)
        .expect("write frontend compiler dependencies");
    // env! records this value in rustc's dependency info, which sccache hashes.
    println!(
        "cargo:rustc-env=HALOGEN_FRONTEND_DIGEST={:x}",
        digest.finalize()
    );
}

fn hash_dir(root: &Path, dir: &Path, digest: &mut Sha256, dependencies: &mut String) {
    let mut entries = std::fs::read_dir(dir)
        .expect("read frontend directory")
        .map(|entry| entry.expect("read frontend entry").path())
        .collect::<Vec<_>>();
    entries.sort();
    for path in entries {
        println!("cargo:rerun-if-changed={}", path.display());
        if path.is_dir() {
            hash_dir(root, &path, digest, dependencies);
        } else {
            use std::fmt::Write as _;
            let absolute = path.canonicalize().expect("resolve frontend asset");
            writeln!(
                dependencies,
                "const _: &[u8] = include_bytes!({:?});",
                absolute
            )
            .expect("record frontend compiler dependency");
            let relative = path.strip_prefix(root).expect("frontend-relative path");
            let name = relative.to_str().expect("UTF-8 frontend asset path");
            let bytes = std::fs::read(&path).expect("read frontend asset");
            digest.update((name.len() as u64).to_le_bytes());
            digest.update(name.as_bytes());
            digest.update((bytes.len() as u64).to_le_bytes());
            digest.update(bytes);
        }
    }
}
