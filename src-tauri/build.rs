fn main() {
    // Re-embed the frontend whenever the built dist changes. tauri-build watches
    // ../dist as a directory, which can miss content-only rewrites of nested
    // files (vite's hashed assets); this guarantees a stale frontend can never
    // survive a `pnpm tauri:build`.
    println!("cargo:rerun-if-changed=../dist");
    tauri_build::build()
}
