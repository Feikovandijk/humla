import { useEffect, useRef } from "react";
import { X } from "lucide-react";
import { Settings } from "../Settings";

// Route-backed settings screen. `/settings?tab=` stays the source of truth
// for what's shown; App renders this full-window over the remembered
// background view instead of swapping the page out.
export function SettingsDialog({ onClose }: { onClose: () => void }) {
  const panelRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Focus management: keyboard/AT users land inside the screen on open and
  // return to whatever triggered it on close.
  useEffect(() => {
    const trigger = document.activeElement;
    panelRef.current?.focus();
    return () => {
      if (trigger instanceof HTMLElement && trigger.isConnected) {
        trigger.focus();
      }
    };
  }, []);

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Settings"
      className="fixed inset-0 z-50 overflow-hidden bg-[var(--color-canvas)]"
    >
      {/* Drag strip: the screen covers the shell's own title-bar handle, so it
          brings its own — the frameless window stays movable and the macOS
          traffic lights stay grabbable at the top-left. */}
      <div
        data-tauri-drag-region
        className="absolute top-0 left-0 right-0 h-9 z-10"
      />
      <button
        type="button"
        onClick={onClose}
        aria-label="Close settings"
        title="Close (Esc)"
        className="no-drag absolute top-3 right-3 z-20 grid place-items-center w-8 h-8 rounded-full text-[var(--color-text-muted)] hover:bg-[var(--color-pill-hover)] hover:text-[var(--color-text)] transition-colors"
      >
        <X size={16} strokeWidth={1.8} />
      </button>
      <div ref={panelRef} tabIndex={-1} className="h-full outline-none">
        <Settings />
      </div>
    </div>
  );
}
