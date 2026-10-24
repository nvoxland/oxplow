fn main() {
    // A rebuilt frontend re-embeds on the next cargo build; no `cargo clean` needed.
    println!("cargo:rerun-if-changed=../dist");
    tauri_build::build()
}
