// What Purl tells people about reading the field it pasted into.
//
// The window must match the correction watch in src-tauri/src/lib.rs:
// WATCH_MAX_POLLS (150) × WATCH_POLL (600 ms) = up to 90 s, ending sooner
// once edits settle (WATCH_SETTLED_POLLS × WATCH_POLL = 12 s quiet). Change
// them together; index.html's Learn corrections help repeats this sentence.
// Understating what is read would be a consent defect.

export const FIELD_READING_WINDOW = "for up to 90 seconds after pasting (less once you stop editing)";

export const FIELD_READING_DISCLOSURE =
  `It also lets Purl read that text field ${FIELD_READING_WINDOW}, to learn words you fix by hand. ` +
  "Password fields are never read, and you can turn this off in Settings.";
