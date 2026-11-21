import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./WindowControls.css";

export function WindowControls() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const appWindow = getCurrentWindow();
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const update = () => {
      void appWindow.isMaximized().then((value) => {
        if (!disposed) setMaximized(value);
      }).catch(console.error);
    };

    update();
    void appWindow.onResized(update).then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    }).catch(console.error);
    return () => { disposed = true; unlisten?.(); };
  }, []);

  const appWindow = getCurrentWindow();
  const toggleMaximize = () => {
    void appWindow.toggleMaximize()
      .then(() => appWindow.isMaximized())
      .then(setMaximized)
      .catch(console.error);
  };

  return (
    <>
      <div
        className="project-titlebar-drag"
        title="Drag to move TaskBoard · double-click to maximize"
        onMouseDown={(event) => {
          if (event.button === 0 && event.detail === 1)
            void appWindow.startDragging().catch(console.error);
        }}
        onDoubleClick={toggleMaximize}
      />
      <div className="project-window-controls" aria-label="Window controls">
        <button type="button" title="Minimize" aria-label="Minimize TaskBoard"
          onClick={() => void appWindow.minimize().catch(console.error)}>
          <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 8.5h10" /></svg>
        </button>
        <button type="button" title={maximized ? "Restore" : "Maximize"}
          aria-label={maximized ? "Restore TaskBoard" : "Maximize TaskBoard"}
          onClick={toggleMaximize}>
          {maximized
            ? <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M5.5 3h7v7h-2M3 5.5h7v7H3z" /></svg>
            : <svg viewBox="0 0 16 16" aria-hidden="true"><rect x="3" y="3" width="10" height="10" rx=".5" /></svg>}
        </button>
        <button type="button" className="project-window-close" title="Close"
          aria-label="Close TaskBoard"
          onClick={() => void appWindow.close().catch(console.error)}>
          <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4 4l8 8m0-8-8 8" /></svg>
        </button>
      </div>
    </>
  );
}
