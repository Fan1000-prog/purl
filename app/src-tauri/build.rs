fn main() {
    stage_onnxruntime_dll();
    tauri_build::build()
}

/// Windows links Microsoft's CPU-only ONNX Runtime dynamically (CI sets
/// `ORT_PREFER_DYNAMIC_LINK=1` and `ORT_LIB_PATH`), so `onnxruntime.dll` must
/// ship beside `purl.exe`. Copy it into `ort-libs/`, which
/// `tauri.windows.conf.json` bundles into the install dir. Windows resolves
/// an import from the exe's own directory before System32, so this copy
/// wins over the older `onnxruntime.dll` some Windows versions ship.
///
/// On Windows the folder always exists so the bundle step has something to
/// read. It is emptied first, so a DLL from an earlier dynamic build never
/// ships with a static one. Other targets link ONNX Runtime statically and
/// skip this.
fn stage_onnxruntime_dll() {
    use std::path::PathBuf;

    for var in [
        "ORT_LIB_PATH",
        "ORT_LIB_LOCATION",
        "ORT_PREFER_DYNAMIC_LINK",
    ] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let staging = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("ort-libs");
    match std::fs::remove_dir_all(&staging) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            panic!("clear {}: {e}", staging.display())
        }
        _ => {}
    }
    std::fs::create_dir_all(&staging).expect("create ort-libs staging dir");
    if !prefers_dynamic_link() {
        return;
    }
    // `ORT_LIB_PATH` is what current `ort` reads; `ORT_LIB_LOCATION` is its
    // older name.
    let Some(lib_dir) =
        std::env::var_os("ORT_LIB_PATH").or_else(|| std::env::var_os("ORT_LIB_LOCATION"))
    else {
        return;
    };
    let src = PathBuf::from(lib_dir).join("onnxruntime.dll");
    println!("cargo:rerun-if-changed={}", src.display());
    std::fs::copy(&src, staging.join("onnxruntime.dll"))
        .unwrap_or_else(|e| panic!("copy {} into ort-libs: {e}", src.display()));
}

/// `ORT_PREFER_DYNAMIC_LINK` counts only when it is `1` or `true` (any case),
/// so `0` or `false` doesn't switch dynamic linking on. Matches how `ort-sys`
/// reads it, so the DLL is staged exactly when `ort` links it.
fn prefers_dynamic_link() -> bool {
    std::env::var("ORT_PREFER_DYNAMIC_LINK").is_ok_and(|v| v == "1" || v.to_lowercase() == "true")
}
