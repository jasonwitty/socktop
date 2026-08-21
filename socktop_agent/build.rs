fn main() {
    println!("cargo:rerun-if-changed=proto/processes.proto");

    // Compile protobuf definitions for processes
    let mut cfg = prost_build::Config::new();
    cfg.out_dir(std::env::var("OUT_DIR").unwrap());
    // Vendored protoc for reproducible builds where available. It ships no
    // riscv64 binary, so on such hosts fall through to $PROTOC / PATH
    // (prost-build's default lookup) — apt: protobuf-compiler.
    if let Ok(protoc) = protoc_bin_vendored::protoc_bin_path() {
        cfg.protoc_executable(protoc);
    }
    // Use local path (ensures file is inside published crate tarball)
    cfg.compile_protos(&["proto/processes.proto"], &["proto"]) // relative to CARGO_MANIFEST_DIR
        .expect("compile protos");
}
