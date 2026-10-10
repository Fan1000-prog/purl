fn main() {
    stage_onnxruntime_dll();
    tauri_build::build()
}

/// Windows links Microsoft's CPU-only ONNX Runtime dynamically (CI sets
/// `ORT_PREFER_DYNAMIC_LINK` and `ORT_LIB_LOCATION`), so `onnxruntime.dll`
/// must ship beside `purl.exe`. Copy it into `ort-libs/`, which
/// `tauri.windows.conf.json` bundles into the install dir. Windows resolves
/// an import from the exe's own directory before System32, so this copy
/// wins over the older `onnxruntime.dll` some Windows versions ship.
///
/// On Windows the folder always exists so the bundle step has something to
/// read; without the env vars it stays empty (the static-link dev build).
/// Other targets link ONNX Runtime statically and skip this.
fn stage_onnxruntime_dll() {
    use std::path::PathBuf;

    println!("cargo:rerun-if-env-changed=ORT_LIB_LOCATION");
    println!("cargo:rerun-if-env-changed=ORT_PREFER_DYNAMIC_LINK");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let staging = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("ort-libs");
    std::fs::create_dir_all(&staging).expect("create ort-libs staging dir");
    if std::env::var_os("ORT_PREFER_DYNAMIC_LINK").is_none() {
        return;
    }
    let Some(lib_dir) = std::env::var_os("ORT_LIB_LOCATION") else {
        return;
    };
    let src = PathBuf::from(lib_dir).join("onnxruntime.dll");
    println!("cargo:rerun-if-changed={}", src.display());
    std::fs::copy(&src, staging.join("onnxruntime.dll"))
        .unwrap_or_else(|e| panic!("copy {} into ort-libs: {e}", src.display()));
}
