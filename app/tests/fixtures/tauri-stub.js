// Minimal stand-in for the Tauri IPC bridge so pages that call
// `invoke` / `listen` load in a plain browser. Tests drive the page with
// window.__emit(event, payload), read calls from window.__invoked, and can
// set window.__results[cmd] to what a command returns (default null), or
// window.__rejects[cmd] to make it fail.
(() => {
  const callbacks = new Map();
  const listeners = new Map();
  let nextId = 1;
  window.__invoked = [];
  window.__TAURI_INTERNALS__ = {
    transformCallback(cb) {
      const id = nextId++;
      callbacks.set(id, cb);
      return id;
    },
    invoke(cmd, args) {
      if (cmd === "plugin:event|listen") {
        const list = listeners.get(args.event) ?? [];
        list.push(args.handler);
        listeners.set(args.event, list);
        return Promise.resolve(nextId++);
      }
      window.__invoked.push({ cmd, args });
      if (window.__rejects?.[cmd]) return Promise.reject(new Error(`${cmd} failed`));
      return Promise.resolve(window.__results?.[cmd] ?? null);
    },
  };
  window.__emit = (event, payload) => {
    for (const id of listeners.get(event) ?? []) callbacks.get(id)?.({ event, id, payload });
  };
})();
