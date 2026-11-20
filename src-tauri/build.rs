fn main() {
    // ONNX Runtime's Apple builds compile CoreML execution-provider objects
    // (model.mm et al.) that reference the CoreML and Metal frameworks; the
    // final bin link line must carry them or arm64 linking fails with
    // "symbol(s) not found" (first hit when embeddings-ort was enabled for
    // shipped builds, v1.0.2).
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-arg=-framework");
        println!("cargo:rustc-link-arg=CoreML");
        println!("cargo:rustc-link-arg=-framework");
        println!("cargo:rustc-link-arg=Metal");
    }
    tauri_build::build()
}
