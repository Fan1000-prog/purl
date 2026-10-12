// What Purl tells people about reading the field it pasted into.
//
// The window must match the correction watch in src-tauri/src/lib.rs:
// WATCH_MAX_POLLS (150) × WATCH_POLL (600 ms) = up to 90 s, ending sooner
// once edits settle (WATCH_SETTLED_POLLS × WATCH_POLL = 12 s quiet). Change
// them together. Every place the app states the window is built from here
// (setup, Settings → Learn corrections, the what's-new card), so index.html
// holds no copy. Understating what is read would be a consent defect.

const UP_TO = "up to 90 seconds";
const SOONER = "(less once you stop editing)";

export const FIELD_READING_WINDOW = `for ${UP_TO} after pasting ${SOONER}`;

/** Settings → Learn corrections help text. */
export const LEARN_CORRECTIONS_HELP =
  `After pasting, Purl reads that text field for ${UP_TO} ${SOONER} to notice words you fix by hand. ` +
  "Only the fixed word and its neighbours are kept, on this Mac. Password fields are never read.";

/** The what's-new card's learning paragraph: a bold lead, then the body. */
export const WHATS_NEW_LEARNING = {
  lead: "Purl learns from you.",
  body:
    " Fix a misheard word right after dictating and it spells it your way next time. " +
    `To notice the fix, Purl reads the text field it pasted into ${FIELD_READING_WINDOW}. ` +
    "Password fields are never read, and you can turn this off in Settings.",
};

export const FIELD_READING_DISCLOSURE =
  `It also lets Purl read that text field ${FIELD_READING_WINDOW}, to learn words you fix by hand. ` +
  "Password fields are never read, and you can turn this off in Settings.";
