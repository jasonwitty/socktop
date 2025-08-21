fn main() {
    // Ensure protoc exists (vendored for reproducible builds)
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("protoc");
    std::env::set_var("PROTOC", protoc);

    // Compile protobuf definitions for processes
    let mut cfg = prost_build::Config::new();
    cfg.out_dir(std::env::var("OUT_DIR").unwrap());
    cfg.compile_protos(&["../proto/processes.proto"], &["../proto"])
        .expect("compile protos");
}
