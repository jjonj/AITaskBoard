// Small stroke icons for the top toolbar and the More menu.
// All use currentColor so they follow button/menu text colour.
type IconName = "undo" | "refresh" | "collapse" | "expand" | "more" | "column" | "releases"
  | "agents" | "blockers" | "user" | "dump" | "folder" | "plus";

const PATHS: Record<IconName, string> = {
  undo: "M7 5 3.5 8.5 7 12M4 8.5h7.5a4.5 4.5 0 0 1 0 9H9",
  refresh: "M16 10a6 6 0 1 1-1.8-4.3M16.5 3.5v3.7h-3.7",
  collapse: "M6 8l4-4 4 4M6 16l4-4 4 4",
  expand: "M6 4l4 4 4-4M6 12l4 4 4-4",
  more: "M4.5 10h.01M10 10h.01M15.5 10h.01",
  column: "M3.5 4h5v12h-5zM11.5 4h5v7h-5zM14 13.5v4M12 15.5h4",
  releases: "M10 2.5 16.5 6v8L10 17.5 3.5 14V6zM3.5 6 10 9.5 16.5 6M10 9.5v8",
  agents: "M10 12.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5zM10 2.5v2M10 15.5v2M2.5 10h2M15.5 10h2M4.7 4.7l1.4 1.4M13.9 13.9l1.4 1.4M4.7 15.3l1.4-1.4M13.9 6.1l1.4-1.4",
  blockers: "M10 3 17.5 16.5h-15zM10 8.5v3.5M10 14.5h.01",
  user: "M10 10a3.2 3.2 0 1 0 0-6.4 3.2 3.2 0 0 0 0 6.4zM3.8 17c.8-3 3.3-4.6 6.2-4.6s5.4 1.6 6.2 4.6",
  dump: "M4 5h12M4 9h12M4 13h8M4 17h5",
  folder: "M2.5 5.5h5l1.5 2h8.5v8.5h-15z",
  plus: "M10 4v12M4 10h12",
};

export function ToolbarIcon({ name, size = 15 }: { name: IconName; size?: number }) {
  return (
    <svg viewBox="0 0 20 20" width={size} height={size} aria-hidden="true" fill="none"
      stroke="currentColor" strokeWidth={name === "more" ? 3 : 1.7} strokeLinecap="round" strokeLinejoin="round">
      <path d={PATHS[name]} />
    </svg>
  );
}
