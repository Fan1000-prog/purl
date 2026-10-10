//! App settings, persisted as JSON in the app config dir.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum InjectMode {
    /// Paste at the cursor via Cmd-V (needs Accessibility permission).
    Paste,
    /// Copy to clipboard only; the user pastes manually.
    ClipboardOnly,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CleanupMode {
    /// Instant rule-based pass: strip filler words, fix glossary casing.
    Fast,
    /// Local LLM cleanup via Ollama — smarter but costs seconds per run.
    Ai,
    /// Paste Whisper's raw output verbatim.
    Off,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AiEngine {
    /// Local Ollama model — nothing leaves the machine (default).
    Local,
    /// Anthropic Claude, using the user's own API key. Opt-in: transcript
    /// text leaves the machine, so this must be an explicit choice.
    Claude,
    /// Google Gemini, using the user's own API key. Same tradeoff.
    Gemini,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StructureMode {
    /// Paste the transcript as spoken.
    None,
    /// Reshape into an email layout: greeting, body paragraphs, sign-off.
    Email,
    /// Prompt/technical dictation: markdown bullets, balanced code fences,
    /// and (with AI cleanup) preserved technical casing and jargon.
    Coding,
}

/// The only two models the picker offers and the only two allowed persisted
/// values. Anything else is coerced to DEFAULT_MODEL on load.
pub const DEFAULT_MODEL: &str = "qwen2.5:3b";
pub const ALT_MODEL: &str = "llama3.2:3b";

pub fn is_allowed_model(model: &str) -> bool {
    model == DEFAULT_MODEL || model == ALT_MODEL
}

pub const DEFAULT_CLAUDE_MODEL: &str = "claude-haiku-4-5-20251001";
pub const DEFAULT_GEMINI_MODEL: &str = "gemini-3.5-flash-lite";

/// Cloud model ids are validated, not allowlisted. Google rotates tiers
/// faster than this app ships: three of the ids the picker offers postdate
/// the first draft of this feature, and `gemini-2.0-flash` — once the
/// free-tier workhorse — is now deprecated. A strict list would need a
/// release per rotation, so anything plausibly an id is accepted and a
/// hand-edited settings.json keeps working.
pub fn is_allowed_cloud_model(model: &str) -> bool {
    let m = model.trim();
    !m.is_empty() && m.len() <= 128 && !m.chars().any(char::is_whitespace) && m == model
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub hotkey: String,
    pub ollama_model: String,
    /// Which backend runs AI cleanup and transforms. API keys live in the
    /// Keychain (see `secrets.rs`), never here.
    pub ai_engine: AiEngine,
    /// Model id sent to the Anthropic Messages API when `ai_engine` is Claude.
    pub claude_model: String,
    /// Model id sent to the Gemini API when `ai_engine` is Gemini.
    pub gemini_model: String,
    /// Empty string → default location under Application Support.
    pub whisper_model_path: String,
    pub inject_mode: InjectMode,
    pub cleanup_mode: CleanupMode,
    /// Reshape transcript layout before pasting (e.g. email format).
    pub structure_mode: StructureMode,
    /// Spoken commands ("go down", "press enter") become keystrokes.
    pub command_detection: bool,
    /// Watch the field we pasted into and learn hand-made word corrections.
    /// On by default: the feature is worthless if nobody finds the switch.
    pub learn_corrections: bool,
    /// True once the first-run wizard has been finished or skipped.
    pub onboarding_complete: bool,
    /// True once the user has been told, in the app, that it reads the field
    /// it pasted into. Defaults false so an install that updated into this
    /// version — and so never sees the wizard, the only strong disclosure
    /// surface — gets a one-time notice instead. Set at the same time as
    /// `onboarding_complete` for genuinely new users, who have just read it
    /// in the wizard and should not be told twice.
    pub field_reading_disclosed: bool,
    /// The app version whose "what's new" card the user has already seen.
    /// Empty on a fresh install, which is how a first launch is told apart
    /// from a launch straight after an update: new users get the wizard, and
    /// showing them a changelog for a version they never ran would be noise.
    pub last_seen_version: String,
    /// True once the one-time model-default migration has run, so a deliberate
    /// choice of the alternate model is no longer auto-upgraded on every launch.
    pub model_migrated: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            hotkey: "CmdOrCtrl+Shift+Space".into(),
            ollama_model: "qwen2.5:3b".into(),
            ai_engine: AiEngine::Local,
            claude_model: DEFAULT_CLAUDE_MODEL.into(),
            gemini_model: DEFAULT_GEMINI_MODEL.into(),
            whisper_model_path: String::new(),
            inject_mode: InjectMode::Paste,
            cleanup_mode: CleanupMode::Ai,
            structure_mode: StructureMode::None,
            command_detection: true,
            learn_corrections: true,
            onboarding_complete: false,
            field_reading_disclosed: false,
            last_seen_version: String::new(),
            model_migrated: false,
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Settings {
        let mut settings: Settings = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if !settings.model_migrated {
            // One-time: move existing public-release users off the old default
            // onto the qwen-tuned cleanup, and coerce any other unrecognized
            // tag to the default. A deliberate ALT_MODEL choice made AFTER this
            // migration is respected because the flag is then set.
            if settings.ollama_model == ALT_MODEL || !is_allowed_model(&settings.ollama_model) {
                settings.ollama_model = DEFAULT_MODEL.into();
            }
            settings.model_migrated = true;
        } else if !is_allowed_model(&settings.ollama_model) {
            settings.ollama_model = DEFAULT_MODEL.into();
        }
        // Cloud ids are only sanity-checked; see `is_allowed_cloud_model`.
        if !is_allowed_cloud_model(&settings.claude_model) {
            settings.claude_model = DEFAULT_CLAUDE_MODEL.into();
        }
        if !is_allowed_cloud_model(&settings.gemini_model) {
            settings.gemini_model = DEFAULT_GEMINI_MODEL.into();
        }
        settings
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).unwrap())
    }

    /// The model id for a cloud engine; empty for `Local`, which uses
    /// `ollama_model` instead.
    pub fn cloud_model(&self, engine: AiEngine) -> &str {
        match engine {
            AiEngine::Local => "",
            AiEngine::Claude => &self.claude_model,
            AiEngine::Gemini => &self.gemini_model,
        }
    }

    /// Resolve the whisper model path: explicit setting, or the default
    /// under Application Support. Default is base.en — ~3x faster decode
    /// than small.en with equivalent accuracy on glossary-prompted dictation;
    /// falls back to small.en when only that one is downloaded.
    pub fn resolved_model_path(&self, app_support: &Path) -> PathBuf {
        if self.whisper_model_path.trim().is_empty() {
            let base = app_support.join("models/ggml-base.en.bin");
            if base.is_file() {
                return base;
            }
            let small = app_support.join("models/ggml-small.en.bin");
            if small.is_file() {
                return small;
            }
            base
        } else {
            PathBuf::from(self.whisper_model_path.trim())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_model_validation_is_permissive_but_bounded() {
        assert!(is_allowed_cloud_model("gemini-3.5-flash-lite"));
        assert!(is_allowed_cloud_model("claude-sonnet-5"));
        // Deliberately allowed: an id Google invents after this release ships.
        assert!(is_allowed_cloud_model("gemini-9.9-flash-lite"));
        assert!(!is_allowed_cloud_model(""));
        assert!(!is_allowed_cloud_model("   "));
        assert!(!is_allowed_cloud_model("has a space"));
        assert!(!is_allowed_cloud_model(&"x".repeat(129)));
    }

    #[test]
    fn ai_engine_defaults_to_local_and_round_trips() {
        assert_eq!(Settings::default().ai_engine, AiEngine::Local);
        assert_eq!(Settings::default().gemini_model, DEFAULT_GEMINI_MODEL);
        assert_eq!(Settings::default().claude_model, DEFAULT_CLAUDE_MODEL);

        let path = std::env::temp_dir().join("purl-settings-ai-engine.json");
        let s = Settings {
            ai_engine: AiEngine::Gemini,
            model_migrated: true,
            ..Default::default()
        };
        s.save(&path).unwrap();
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(on_disk.contains(r#""ai_engine": "gemini""#));
        // The engine:model token is UI transport only — never persisted.
        assert!(!on_disk.contains("gemini:gemini"));
        assert_eq!(Settings::load(&path).ai_engine, AiEngine::Gemini);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn legacy_settings_without_ai_engine_load_as_local() {
        let path = std::env::temp_dir().join("purl-settings-legacy-engine.json");
        std::fs::write(
            &path,
            r#"{"hotkey":"CmdOrCtrl+Shift+Space","ollama_model":"qwen2.5:3b","whisper_model_path":"","inject_mode":"paste","cleanup_mode":"ai","model_migrated":true}"#,
        )
        .unwrap();
        let loaded = Settings::load(&path);
        assert_eq!(loaded.ai_engine, AiEngine::Local);
        assert_eq!(loaded.gemini_model, DEFAULT_GEMINI_MODEL);
        assert_eq!(loaded.claude_model, DEFAULT_CLAUDE_MODEL);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn invalid_cloud_model_resets_to_engine_default_on_load() {
        let path = std::env::temp_dir().join("purl-settings-bad-cloud-model.json");
        std::fs::write(
            &path,
            r#"{"ollama_model":"qwen2.5:3b","model_migrated":true,"ai_engine":"gemini","gemini_model":"bad model id"}"#,
        )
        .unwrap();
        assert_eq!(Settings::load(&path).gemini_model, DEFAULT_GEMINI_MODEL);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn selecting_a_cloud_engine_does_not_disturb_the_local_model() {
        // Switching Local -> Gemini -> Local must preserve the local choice,
        // which is why models are stored per engine rather than in one field.
        let path = std::env::temp_dir().join("purl-settings-engine-switch.json");
        let s = Settings {
            ollama_model: ALT_MODEL.into(),
            model_migrated: true,
            ai_engine: AiEngine::Gemini,
            ..Default::default()
        };
        s.save(&path).unwrap();
        let loaded = Settings::load(&path);
        assert_eq!(loaded.ollama_model, ALT_MODEL);
        assert_eq!(loaded.cloud_model(AiEngine::Gemini), DEFAULT_GEMINI_MODEL);
        assert_eq!(loaded.cloud_model(AiEngine::Local), "");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn defaults_are_sane() {
        let s = Settings::default();
        assert_eq!(s.hotkey, "CmdOrCtrl+Shift+Space");
        assert_eq!(s.ollama_model, "qwen2.5:3b");
        assert_eq!(s.cleanup_mode, CleanupMode::Ai);
        assert_eq!(s.inject_mode, InjectMode::Paste);
        assert_eq!(s.structure_mode, StructureMode::None);
        assert!(s.command_detection);
    }

    #[test]
    fn legacy_settings_get_v03_defaults() {
        // A 0.2 settings.json has neither structure_mode nor command_detection.
        let path = std::env::temp_dir().join("purl-settings-v02.json");
        std::fs::write(
            &path,
            r#"{"hotkey":"CmdOrCtrl+Shift+Space","ollama_model":"llama3.2:3b","whisper_model_path":"","inject_mode":"paste","cleanup_mode":"fast","onboarding_complete":true}"#,
        )
        .unwrap();
        let loaded = Settings::load(&path);
        assert_eq!(loaded.structure_mode, StructureMode::None);
        assert!(loaded.command_detection);
        assert!(loaded.onboarding_complete);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn structure_mode_round_trips() {
        let path = std::env::temp_dir().join("purl-settings-structure.json");
        let s = Settings {
            structure_mode: StructureMode::Email,
            command_detection: false,
            ..Default::default()
        };
        s.save(&path).unwrap();
        let loaded = Settings::load(&path);
        assert_eq!(loaded.structure_mode, StructureMode::Email);
        assert!(!loaded.command_detection);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn coding_mode_round_trips_as_snake_case() {
        let path = std::env::temp_dir().join("purl-settings-coding.json");
        let s = Settings {
            structure_mode: StructureMode::Coding,
            ..Default::default()
        };
        s.save(&path).unwrap();
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(on_disk.contains(r#""structure_mode": "coding""#));
        assert_eq!(Settings::load(&path).structure_mode, StructureMode::Coding);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn legacy_settings_without_cleanup_mode_get_ai() {
        let path = std::env::temp_dir().join("purl-settings-legacy.json");
        std::fs::write(
            &path,
            r#"{"hotkey":"CmdOrCtrl+Shift+Space","ollama_model":"llama3.2:3b","whisper_model_path":"","inject_mode":"paste","cleanup_enabled":true}"#,
        )
        .unwrap();
        let loaded = Settings::load(&path);
        assert_eq!(loaded.cleanup_mode, CleanupMode::Ai);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn old_default_model_migrates_to_new_default() {
        let path = std::env::temp_dir().join("purl-settings-model-migration.json");
        std::fs::write(
            &path,
            r#"{"hotkey":"CmdOrCtrl+Shift+Space","ollama_model":"llama3.2:3b","whisper_model_path":"","inject_mode":"paste","cleanup_mode":"fast"}"#,
        )
        .unwrap();
        assert_eq!(Settings::load(&path).ollama_model, "qwen2.5:3b");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn unrecognized_model_resets_to_default_on_first_migration() {
        let path = std::env::temp_dir().join("purl-settings-model-custom.json");
        std::fs::write(
            &path,
            r#"{"hotkey":"CmdOrCtrl+Shift+Space","ollama_model":"qwen3:8b","whisper_model_path":"","inject_mode":"paste","cleanup_mode":"fast"}"#,
        )
        .unwrap();
        assert_eq!(Settings::load(&path).ollama_model, "qwen2.5:3b");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn round_trips_through_disk() {
        let path = std::env::temp_dir().join("purl-settings-test.json");
        let s = Settings {
            ollama_model: "llama3.2:3b".into(),
            model_migrated: true,
            inject_mode: InjectMode::ClipboardOnly,
            ..Default::default()
        };
        s.save(&path).unwrap();
        let loaded = Settings::load(&path);
        assert_eq!(loaded.ollama_model, "llama3.2:3b");
        assert_eq!(loaded.inject_mode, InjectMode::ClipboardOnly);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let path = std::env::temp_dir().join("purl-settings-corrupt.json");
        std::fs::write(&path, "{not json").unwrap();
        let loaded = Settings::load(&path);
        assert_eq!(loaded.hotkey, Settings::default().hotkey);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn model_path_resolution() {
        let s = Settings::default();
        // Neither model file exists at this fake root → base.en (the default).
        let resolved = s.resolved_model_path(Path::new("/tmp/appsupport"));
        assert_eq!(
            resolved,
            PathBuf::from("/tmp/appsupport/models/ggml-base.en.bin")
        );

        // Only small.en on disk (pre-upgrade install) → keep using it.
        let root = std::env::temp_dir().join("purl-model-res-test");
        std::fs::create_dir_all(root.join("models")).unwrap();
        std::fs::write(root.join("models/ggml-small.en.bin"), b"x").unwrap();
        assert_eq!(
            s.resolved_model_path(&root),
            root.join("models/ggml-small.en.bin")
        );
        std::fs::write(root.join("models/ggml-base.en.bin"), b"x").unwrap();
        assert_eq!(
            s.resolved_model_path(&root),
            root.join("models/ggml-base.en.bin")
        );
        std::fs::remove_dir_all(&root).unwrap();
        let s2 = Settings {
            whisper_model_path: "/custom/model.bin".into(),
            ..Default::default()
        };
        assert_eq!(
            s2.resolved_model_path(Path::new("/tmp/appsupport")),
            PathBuf::from("/custom/model.bin")
        );
    }

    #[test]
    fn onboarding_flag_defaults_false_and_round_trips() {
        assert!(!Settings::default().onboarding_complete);

        // Legacy settings file without the field → false, not a parse error.
        let path = std::env::temp_dir().join("purl-settings-onboarding.json");
        std::fs::write(
            &path,
            r#"{"hotkey":"CmdOrCtrl+Shift+Space","ollama_model":"llama3.2:3b","whisper_model_path":"","inject_mode":"paste","cleanup_mode":"fast"}"#,
        )
        .unwrap();
        assert!(!Settings::load(&path).onboarding_complete);

        let s = Settings {
            onboarding_complete: true,
            ..Default::default()
        };
        s.save(&path).unwrap();
        assert!(Settings::load(&path).onboarding_complete);
        std::fs::remove_file(&path).unwrap();
    }

    /// An install that auto-updated into field reading has
    /// `onboarding_complete: true` already, so it never meets the wizard.
    /// The flag must load false for it — that is what triggers the one-time
    /// notice — and true for anyone who has since dismissed it.
    #[test]
    fn field_reading_disclosure_defaults_false_for_existing_installs() {
        assert!(!Settings::default().field_reading_disclosed);

        let path = std::env::temp_dir().join("purl-settings-field-disclosure.json");
        std::fs::write(
            &path,
            r#"{"hotkey":"CmdOrCtrl+Shift+Space","ollama_model":"qwen2.5:3b","whisper_model_path":"","inject_mode":"paste","cleanup_mode":"ai","model_migrated":true,"onboarding_complete":true}"#,
        )
        .unwrap();
        let loaded = Settings::load(&path);
        assert!(loaded.onboarding_complete);
        assert!(!loaded.field_reading_disclosed);

        let mut s = loaded;
        s.field_reading_disclosed = true;
        s.save(&path).unwrap();
        assert!(Settings::load(&path).field_reading_disclosed);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn model_migrated_defaults_false_and_sets_true_on_load() {
        assert!(!Settings::default().model_migrated);
        let path = std::env::temp_dir().join("purl-settings-migflag.json");
        std::fs::write(
            &path,
            r#"{"hotkey":"CmdOrCtrl+Shift+Space","ollama_model":"llama3.2:3b","whisper_model_path":"","inject_mode":"paste","cleanup_mode":"ai"}"#,
        )
        .unwrap();
        assert!(Settings::load(&path).model_migrated);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn deliberate_llama_choice_survives_after_migration() {
        let path = std::env::temp_dir().join("purl-settings-llama-kept.json");
        std::fs::write(
            &path,
            r#"{"hotkey":"CmdOrCtrl+Shift+Space","ollama_model":"llama3.2:3b","whisper_model_path":"","inject_mode":"paste","cleanup_mode":"ai","model_migrated":true}"#,
        )
        .unwrap();
        assert_eq!(Settings::load(&path).ollama_model, "llama3.2:3b");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn unrecognized_model_resets_even_after_migration() {
        let path = std::env::temp_dir().join("purl-settings-invalid-postmig.json");
        std::fs::write(
            &path,
            r#"{"hotkey":"CmdOrCtrl+Shift+Space","ollama_model":"qwen3:8b","whisper_model_path":"","inject_mode":"paste","cleanup_mode":"ai","model_migrated":true}"#,
        )
        .unwrap();
        assert_eq!(Settings::load(&path).ollama_model, "qwen2.5:3b");
        std::fs::remove_file(&path).unwrap();
    }
}
