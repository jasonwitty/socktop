fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Vendored protoc for reproducible builds where available. It ships no
    // riscv64 binary, so on such hosts leave $PROTOC / PATH lookup to
    // prost-build (apt: protobuf-compiler).
    // SAFETY: We're only setting PROTOC in a build script environment.
    if let Ok(protoc) = protoc_bin_vendored::protoc_bin_path() {
        unsafe {
            std::env::set_var("PROTOC", protoc);
        }
    }

    prost_build::compile_protos(&["processes.proto"], &["."])?;
    Ok(())
}
