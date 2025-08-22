fn main() {
    // Vendored protoc for reproducible builds
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("protoc");
    std::env::set_var("PROTOC", &protoc);

    println!("cargo:rerun-if-changed=proto/processes.proto");

    // Compile protobuf definitions for processes
    let mut cfg = prost_build::Config::new();
    cfg.out_dir(std::env::var("OUT_DIR").unwrap());
    // Use local path (ensures file is inside published crate tarball)
    cfg.compile_protos(&["proto/processes.proto"], &["proto"]) // relative to CARGO_MANIFEST_DIR
        .expect("compile protos");
}
