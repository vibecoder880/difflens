fn main() {
    #[cfg(target_os = "macos")]
    {
        // ONNX Runtime's Apple builds compile CoreML execution-provider
        // objects (model.mm et al.) that reference the CoreML and Metal
        // frameworks; the final bin link line must carry them or arm64
        // linking fails with "symbol(s) not found" (first hit when
        // embeddings-ort was enabled for shipped builds, v1.0.2).
        println!("cargo:rustc-link-arg=-framework");
        println!("cargo:rustc-link-arg=CoreML");
        println!("cargo:rustc-link-arg=-framework");
        println!("cargo:rustc-link-arg=Metal");
        // ort-sys objects are built against macOS 13.4 and reference
        // ___isPlatformVersionAtLeast, which the compiler-rt variant for
        // rustc's default aarch64-apple-darwin min (11.0) does not export.
        // ld honors the LAST -macosx_version_min on the line; bin link
        // args land at the end, so this overrides rustc's 11.0 default.
        println!("cargo:rustc-link-arg=-Wl,-macosx_version_min,13.0");
    }
    tauri_build::build()
}
