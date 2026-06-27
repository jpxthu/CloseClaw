use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let platforms_dir = Path::new("src/platforms");
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();

    // Re-run when the platforms directory changes.
    println!("cargo:rerun-if-changed=src/platforms");

    let mut mods = Vec::new();

    if let Ok(entries) = fs::read_dir(platforms_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name() {
                    if let Some(name_str) = name.to_str() {
                        // Use #[path] with absolute path so module resolution
                        // works correctly when included via include!() from
                        // a different file context (OUT_DIR).
                        let abs_path =
                            format!("{}/src/platforms/{}/mod.rs", manifest_dir, name_str);
                        mods.push(format!("#[path = \"{}\"]\npub mod {};", abs_path, name_str));
                    }
                }
            }
        }
    }

    mods.sort();
    let content = mods.join("\n") + "\n";

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("platforms_gen.rs");
    fs::write(dest_path, content).unwrap();
}
