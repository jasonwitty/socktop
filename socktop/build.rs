fn main() {
    // Vendored protoc for reproducible builds (works on crates.io build machines)
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("protoc");
    std::env::set_var("PROTOC", &protoc);

    // Tell Cargo when to re-run
    println!("cargo:rerun-if-changed=proto/processes.proto");

    let mut cfg = prost_build::Config::new();
    cfg.out_dir(std::env::var("OUT_DIR").unwrap());
    // Use in-crate relative path so `cargo package` includes the file
    cfg.compile_protos(&["proto/processes.proto"], &["proto"]) // paths relative to CARGO_MANIFEST_DIR
        .expect("compile protos");
}
