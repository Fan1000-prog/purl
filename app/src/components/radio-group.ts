// Keyboard behaviour for the segmented controls (role=radiogroup of
// role=radio buttons), per the ARIA radio group pattern: the group is one
// Tab stop (tabindex 0 on the checked button, -1 on the rest), and the arrow
// keys move focus and select at once, wrapping at the ends.

const NEXT: Record<string, number> = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 };

function radios(group: HTMLElement): HTMLButtonElement[] {
  return [...group.querySelectorAll<HTMLButtonElement>("button[role=radio]")];
}

/** Check the button `isChecked` picks and give it the group's Tab stop.
 *  With none checked, the first button keeps the group reachable. */
export function setRadio(group: HTMLElement, isChecked: (b: HTMLButtonElement) => boolean): void {
  const all = radios(group);
  const checked = all.find(isChecked) ?? null;
  for (const b of all) {
    b.setAttribute("aria-checked", String(b === checked));
    b.tabIndex = b === (checked ?? all[0]) ? 0 : -1;
  }
}

/** Call `select` on click, or on an arrow key after moving focus there.
 *  `select` is expected to re-render the group through setRadio. */
export function onRadio(group: HTMLElement, select: (b: HTMLButtonElement) => void): void {
  for (const b of radios(group)) b.addEventListener("click", () => select(b));
  group.addEventListener("keydown", (e) => {
    const step = NEXT[e.key];
    if (!step) return;
    const enabled = radios(group).filter((b) => !b.disabled);
    const at = enabled.indexOf(e.target as HTMLButtonElement);
    if (at < 0) return;
    e.preventDefault();
    const next = enabled[(at + step + enabled.length) % enabled.length];
    next.focus();
    select(next);
  });
}
