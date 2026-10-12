// Two-step confirm on the button itself: the first click arms it and asks,
// a second click within 4 s runs the action. WKWebView can silently no-op
// window.confirm(), so destructive actions use this instead.

const ARM_MS = 4000;

export function confirmButton(btn: HTMLButtonElement, question: string, action: () => void): void {
  const label = btn.textContent ?? "";
  let timer: ReturnType<typeof setTimeout> | undefined;

  const disarm = () => {
    clearTimeout(timer);
    timer = undefined;
    btn.textContent = label;
    btn.classList.remove("armed");
  };

  btn.addEventListener("click", () => {
    if (timer === undefined) {
      btn.textContent = question;
      btn.classList.add("armed");
      timer = setTimeout(disarm, ARM_MS);
      return;
    }
    disarm();
    action();
  });
}
