use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let platforms_dir = Path::new("src/im_adapter/platforms");

    // Re-run when the platforms directory changes.
    println!("cargo:rerun-if-changed=src/im_adapter/platforms");

    let mut mods = Vec::new();

    if let Ok(entries) = fs::read_dir(platforms_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name() {
                    if let Some(name_str) = name.to_str() {
                        mods.push(format!("pub mod {};", name_str));
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
