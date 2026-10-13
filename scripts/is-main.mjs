// Whether a script was run directly (`node scripts/x.mjs`, by any path,
// symlinks included) rather than imported by another module or a test.
import { realpathSync } from "node:fs";
import { fileURLToPath } from "node:url";

/** Node resolves symlinks in `import.meta.url` but not in `process.argv[1]`,
 *  so both sides are compared by real path. False when there is no script
 *  path (`node -e`, the REPL) or it can't be resolved. */
export function isMain(moduleUrl, scriptPath = process.argv[1]) {
  if (!scriptPath) return false;
  try {
    return realpathSync(scriptPath) === realpathSync(fileURLToPath(moduleUrl));
  } catch {
    return false;
  }
}
