//! App settings, persisted as JSON in the app config dir.

use serde::{Deserialize, Serialize};
use std::path::Path;

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
    /// Paste the transcript verbatim.
    Off,
    /// Instant rule-based pass: fillers, punctuation, dictionary casing.
    Fast,
    /// Local LLM via the bundled llama-server. Falls back to Fast on any failure.
    Smart,
    /// Pre-Purl value; read as Fast so old settings files still load.
    #[serde(alias = "ai")]
    #[serde(skip_serializing)]
    LegacyAi,
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

/// The Smart cleanup models a user can pick. Anything else is coerced to
/// DEFAULT_SMART_MODEL on load.
pub const SMART_MODELS: [&str; 2] = ["qwen3.5-2b", "qwen3.5-4b"];
pub const DEFAULT_SMART_MODEL: &str = "qwen3.5-2b";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub hotkey: String,
    pub inject_mode: InjectMode,
    pub cleanup_mode: CleanupMode,
    /// Which model Smart cleanup runs; one of `SMART_MODELS`.
    pub smart_model: String,
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
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            hotkey: "CmdOrCtrl+Shift+Space".into(),
            inject_mode: InjectMode::Paste,
            cleanup_mode: CleanupMode::Fast,
            smart_model: DEFAULT_SMART_MODEL.into(),
            structure_mode: StructureMode::None,
            command_detection: true,
            learn_corrections: true,
            onboarding_complete: false,
            field_reading_disclosed: false,
            last_seen_version: String::new(),
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Settings {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<Settings>(&s).ok())
            .unwrap_or_default()
            .sanitized()
    }

    /// Coerce values this version no longer accepts. Applied on load and on
    /// every save from the UI, so a legacy or hand-edited value never runs.
    pub fn sanitized(mut self) -> Settings {
        if self.cleanup_mode == CleanupMode::LegacyAi {
            self.cleanup_mode = CleanupMode::Fast;
        }
        if !SMART_MODELS.contains(&self.smart_model.as_str()) {
            self.smart_model = DEFAULT_SMART_MODEL.into();
        }
        self
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let s = Settings::default();
        assert_eq!(s.hotkey, "CmdOrCtrl+Shift+Space");
        assert_eq!(s.cleanup_mode, CleanupMode::Fast);
        assert_eq!(s.smart_model, DEFAULT_SMART_MODEL);
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
    fn legacy_settings_without_cleanup_mode_get_fast() {
        let path = std::env::temp_dir().join("purl-settings-legacy.json");
        std::fs::write(
            &path,
            r#"{"hotkey":"CmdOrCtrl+Shift+Space","ollama_model":"llama3.2:3b","whisper_model_path":"","inject_mode":"paste","cleanup_enabled":true}"#,
        )
        .unwrap();
        let loaded = Settings::load(&path);
        assert_eq!(loaded.cleanup_mode, CleanupMode::Fast);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn round_trips_through_disk() {
        let path = std::env::temp_dir().join("purl-settings-test.json");
        let s = Settings {
            cleanup_mode: CleanupMode::Smart,
            smart_model: "qwen3.5-4b".into(),
            inject_mode: InjectMode::ClipboardOnly,
            ..Default::default()
        };
        s.save(&path).unwrap();
        let loaded = Settings::load(&path);
        assert_eq!(loaded.cleanup_mode, CleanupMode::Smart);
        assert_eq!(loaded.smart_model, "qwen3.5-4b");
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
    fn legacy_ai_mode_loads_as_fast() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.json");
        std::fs::write(
            &p,
            r#"{"cleanup_mode":"ai","ai_engine":"claude","ollama_model":"qwen2.5:3b"}"#,
        )
        .unwrap();
        let s = Settings::load(&p);
        assert_eq!(s.cleanup_mode, CleanupMode::Fast);
        assert_eq!(s.smart_model, "qwen3.5-2b");
    }

    #[test]
    fn unknown_smart_model_is_coerced() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.json");
        std::fs::write(&p, r#"{"cleanup_mode":"smart","smart_model":"gpt-9"}"#).unwrap();
        assert_eq!(Settings::load(&p).smart_model, "qwen3.5-2b");
    }

    /// Removed fields must not be written back: a saved file carries no
    /// trace of the cloud engines or Ollama.
    #[test]
    fn saved_settings_carry_no_legacy_fields() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.json");
        std::fs::write(
            &p,
            r#"{"cleanup_mode":"ai","ai_engine":"gemini","gemini_model":"x","ollama_model":"qwen2.5:3b","model_migrated":true}"#,
        )
        .unwrap();
        Settings::load(&p).save(&p).unwrap();
        let on_disk = std::fs::read_to_string(&p).unwrap();
        for gone in [
            "ai_engine",
            "gemini",
            "claude",
            "ollama",
            "whisper_model_path",
            "model_migrated",
        ] {
            assert!(!on_disk.contains(gone), "{gone} survived a save: {on_disk}");
        }
        assert!(on_disk.contains(r#""cleanup_mode": "fast""#));
    }
}
