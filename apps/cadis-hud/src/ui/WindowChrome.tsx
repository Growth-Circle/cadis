/**
 * Frameless window chrome — replaces the OS title bar.
 *
 *   - whole bar is a tauri drag region (drag the window by it)
 *   - pin button toggles always-on-top
 *   - minimize / close buttons (only enabled in Tauri context)
 */
import { useEffect, useState } from "react";
import { useHud } from "./hudState.js";
import { persistAlwaysOnTopPreference } from "./cadisActions.js";

type WindowApi = {
  setAlwaysOnTop: (v: boolean) => Promise<void>;
  minimize: () => Promise<void>;
  close: () => Promise<void>;
};

async function getWindow(): Promise<WindowApi | null> {
  try {
    const mod = await import("@tauri-apps/api/window");
    const win = mod.getCurrentWindow();
    return {
      setAlwaysOnTop: (v) => win.setAlwaysOnTop(v),
      minimize: () => win.minimize(),
      close: () => win.close(),
    };
  } catch {
    return null;
  }
}

export function WindowChrome() {
  const [pinned, setPinned] = useState(false);
  const [api, setApi] = useState<WindowApi | null>(null);
  const openConfig = useHud((s) => s.setConfigOpen);

  useEffect(() => {
    getWindow().then(setApi);
  }, []);

  const togglePin = async () => {
    if (!api) return;
    const next = !pinned;
    await api.setAlwaysOnTop(next);
    setPinned(next);
    persistAlwaysOnTopPreference(next);
  };

  return (
    <div className="window-chrome" data-tauri-drag-region>
      <div className="window-chrome__handle" data-tauri-drag-region>
        <span className="window-chrome__grip" data-tauri-drag-region>·  ·  ·</span>
        <span className="window-chrome__title" data-tauri-drag-region>CADIS</span>
      </div>
      <div className="window-chrome__buttons">
        <button
          type="button"
          className="window-chrome__btn"
          onClick={() => openConfig(true, "window")}
          title="Window configure"
          aria-label="window configure"
        >
          ⚙
        </button>
        <button
          type="button"
          className={`window-chrome__btn${pinned ? " window-chrome__btn--active" : ""}`}
          onClick={togglePin}
          title={pinned ? "Unpin (allow other windows on top)" : "Pin (always on top)"}
          aria-label="toggle always on top"
        >
          {pinned ? "⊙" : "○"}
        </button>
        <button
          type="button"
          className="window-chrome__btn"
          onClick={() => api?.minimize()}
          title="Minimize"
          aria-label="minimize"
        >
          ▁
        </button>
        <button
          type="button"
          className="window-chrome__btn window-chrome__btn--close"
          onClick={() => api?.close()}
          title="Close"
          aria-label="close"
        >
          ×
        </button>
      </div>
    </div>
  );
}
