fn main() {
    println!("cargo:rerun-if-changed=web/dist");
    if !std::path::Path::new("web/dist/index.html").exists() {
        panic!(
            "Missing frontend build: run npm ci --prefix web && npm run build --prefix web first"
        );
    }
}
