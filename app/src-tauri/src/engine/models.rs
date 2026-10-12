//! The model registry: every model Purl can download, pinned by URL, size
//! and sha256, plus where each one lives on disk.

use std::path::{Path, PathBuf};

pub struct ModelFile {
    pub name: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

pub struct ModelSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub kind: Kind,
    pub files: &'static [ModelFile],
    pub licence: &'static str,
}

impl ModelSpec {
    /// Total bytes across every file.
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Stt,
    Llm,
}

/// Parakeet files all live under one Hugging Face repo.
macro_rules! parakeet_url {
    ($name:literal) => {
        concat!(
            "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/",
            $name
        )
    };
}

static PARAKEET_FILES: [ModelFile; 5] = [
    ModelFile {
        name: "encoder-model.int8.onnx",
        url: parakeet_url!("encoder-model.int8.onnx"),
        size: 652_183_999,
        sha256: "6139d2fa7e1b086097b277c7149725edbab89cc7c7ae64b23c741be4055aff09",
    },
    ModelFile {
        name: "decoder_joint-model.int8.onnx",
        url: parakeet_url!("decoder_joint-model.int8.onnx"),
        size: 18_202_004,
        sha256: "eea7483ee3d1a30375daedc8ed83e3960c91b098812127a0d99d1c8977667a70",
    },
    ModelFile {
        name: "nemo128.onnx",
        url: parakeet_url!("nemo128.onnx"),
        size: 139_764,
        sha256: "a9fde1486ebfcc08f328d75ad4610c67835fea58c73ba57e3209a6f6cf019e9f",
    },
    ModelFile {
        name: "vocab.txt",
        url: parakeet_url!("vocab.txt"),
        size: 93_939,
        sha256: "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d",
    },
    ModelFile {
        name: "config.json",
        url: parakeet_url!("config.json"),
        size: 97,
        sha256: "666903c76b9798caf2c210afd4f6cd60b08a8dbf9800ec8d7a3bc0d2148ac466",
    },
];

static QWEN_2B_FILES: [ModelFile; 1] = [ModelFile {
    name: "Qwen_Qwen3.5-2B-Q4_K_M.gguf",
    url: "https://huggingface.co/bartowski/Qwen_Qwen3.5-2B-GGUF/resolve/main/Qwen_Qwen3.5-2B-Q4_K_M.gguf",
    size: 1_396_198_496,
    sha256: "57a1085840f497d764a7fc5d346922dbde961efb54cc792ea81d694fd846a1d8",
}];

static QWEN_4B_FILES: [ModelFile; 1] = [ModelFile {
    name: "Qwen_Qwen3.5-4B-Q4_K_M.gguf",
    url: "https://huggingface.co/bartowski/Qwen_Qwen3.5-4B-GGUF/resolve/main/Qwen_Qwen3.5-4B-Q4_K_M.gguf",
    size: 3_013_027_808,
    sha256: "13c16f426047e2de38cd075bdade4a7bcbc8c774384876f677740cda65f8a983",
}];

static REGISTRY: [ModelSpec; 3] = [
    ModelSpec {
        id: "parakeet-v3",
        label: "Parakeet v3 (speech)",
        kind: Kind::Stt,
        files: &PARAKEET_FILES,
        licence: "CC-BY-4.0",
    },
    ModelSpec {
        id: "qwen3.5-2b",
        label: "Qwen3.5 2B (smart cleanup)",
        kind: Kind::Llm,
        files: &QWEN_2B_FILES,
        licence: "Apache-2.0",
    },
    ModelSpec {
        id: "qwen3.5-4b",
        label: "Qwen3.5 4B (smart cleanup, larger)",
        kind: Kind::Llm,
        files: &QWEN_4B_FILES,
        licence: "Apache-2.0",
    },
];

pub fn registry() -> &'static [ModelSpec] {
    &REGISTRY
}

pub fn find(id: &str) -> Option<&'static ModelSpec> {
    REGISTRY.iter().find(|m| m.id == id)
}

/// `<app_data>/models/<id>/`
pub fn model_dir(app_data: &Path, id: &str) -> PathBuf {
    app_data.join("models").join(id)
}

/// Written next to a file once its sha256 has been checked, holding that
/// sha256. Lets later launches trust the file without re-hashing ~650 MB.
pub fn ok_marker(dir: &Path, file: &ModelFile) -> PathBuf {
    dir.join(format!("{}.ok", file.name))
}

/// True when this file is on disk at its exact size and was verified
/// against its pinned sha256.
pub fn is_file_verified(dir: &Path, file: &ModelFile) -> bool {
    let size_ok = std::fs::metadata(dir.join(file.name))
        .map(|m| m.len() == file.size)
        .unwrap_or(false);
    size_ok
        && std::fs::read_to_string(ok_marker(dir, file))
            .map(|s| s.trim() == file.sha256)
            .unwrap_or(false)
}

/// Every file present at its exact size and verified.
pub fn is_installed(app_data: &Path, spec: &ModelSpec) -> bool {
    is_installed_in(&model_dir(app_data, spec.id), spec)
}

/// `is_installed` for a caller that already holds the model's folder.
pub fn is_installed_in(dir: &Path, spec: &ModelSpec) -> bool {
    spec.files.iter().all(|f| is_file_verified(dir, f))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_knows_every_registry_id() {
        for m in registry() {
            assert_eq!(find(m.id).map(|f| f.id), Some(m.id));
        }
        assert!(find("whisper").is_none());
    }

    #[test]
    fn parakeet_urls_point_at_their_own_file() {
        const HF_PK: &str =
            "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/";
        for f in find("parakeet-v3").unwrap().files {
            assert_eq!(f.url, format!("{HF_PK}{}", f.name));
        }
    }

    #[test]
    fn model_dir_is_per_id_under_models() {
        assert_eq!(
            model_dir(Path::new("/data"), "parakeet-v3"),
            PathBuf::from("/data/models/parakeet-v3")
        );
    }

    #[test]
    fn is_installed_needs_size_and_marker() {
        let app_data = std::env::temp_dir().join(format!("purl-models-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&app_data);
        static FILES: [ModelFile; 1] = [ModelFile {
            name: "a.bin",
            url: "https://example.invalid/a.bin",
            size: 3,
            sha256: "abc",
        }];
        let spec = ModelSpec {
            id: "m",
            label: "M",
            kind: Kind::Stt,
            files: &FILES,
            licence: "MIT",
        };
        let dir = model_dir(&app_data, "m");
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(dir.join("a.bin"), b"xyz").unwrap();
        assert!(!is_installed(&app_data, &spec), "no marker yet");

        std::fs::write(ok_marker(&dir, &FILES[0]), "abc").unwrap();
        assert!(is_installed(&app_data, &spec));

        std::fs::write(dir.join("a.bin"), b"xy").unwrap();
        assert!(!is_installed(&app_data, &spec), "wrong size");

        let _ = std::fs::remove_dir_all(&app_data);
    }
}
