import type { TaskIcon } from "./types";

export const TASK_ICON_OPTIONS: { value: TaskIcon; label: string }[] = [
  { value: "bug", label: "Bug" },
  { value: "feature", label: "New feature" },
  { value: "refactor", label: "Refactor" },
  { value: "content", label: "Content" },
  { value: "validation", label: "Validation" },
  { value: "design", label: "Design" },
  { value: "visual-work", label: "Visual Work" },
];

export function TaskIconGlyph({ icon, size = 19 }: { icon: TaskIcon; size?: number }) {
  const shared = {
    width: size,
    height: size,
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: 1.85,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
    "aria-hidden": true as const,
  };

  switch (icon) {
    case "design":
      return (
        <svg {...shared}>
          <path d="M12 3 5.5 19M12 3l6.5 16M7 15h10M12 3v3" />
          <circle cx="12" cy="3.5" r="1.7" fill="currentColor" stroke="none" />
          <path d="M4 21h16M8 21v-3M16 21v-3" strokeWidth="1.4" />
        </svg>
      );
    case "visual-work":
      return (
        <svg {...shared}>
          <path d="M2 12s4-6 10-6 10 6 10 6-4 6-10 6-10-6-10-6z" />
          <circle cx="12" cy="12" r="3" />
          <path d="m18.5 2 .6 2.4L21.5 5l-2.4.6-.6 2.4-.6-2.4L15.5 5l2.4-.6z" strokeWidth="1.25" />
        </svg>
      );
    case "bug":
      return (
        <svg {...shared}>
          <path d="M9 7 7 4M15 7l2-3M8 10 4.5 8M8 14 4 14M8.5 18l-3 2M16 10l3.5-2M16 14h4M15.5 18l3 2" />
          <path d="M8 11a4 4 0 0 1 8 0v5a4 4 0 0 1-8 0z" />
          <path d="M8 12h8M12 12v8" />
        </svg>
      );
    case "feature":
      return (
        <svg {...shared}>
          <path d="m12 2 2.5 7.5L22 12l-7.5 2.5L12 22l-2.5-7.5L2 12l7.5-2.5z" />
          <path d="m19.5 2 .5 2.5L22.5 5l-2.5.5-.5 2.5-.5-2.5L16.5 5l2.5-.5z" strokeWidth="1.3" />
        </svg>
      );
    case "refactor":
      return (
        <svg {...shared}>
          <path d="M19 8a8 8 0 0 0-13.7-2L3 8m0-5v5h5M5 16a8 8 0 0 0 13.7 2L21 16m0 5v-5h-5" />
          <path d="M9 12h6M12 9l3 3-3 3" strokeWidth="1.6" />
        </svg>
      );
    case "content":
      return (
        <svg {...shared}>
          <path d="M13 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-9zM13 3v6h5M7.5 12h5M7.5 15.5H11" />
          <path d="m13.5 18.5 1-.4 5.1-5.1 1.5 1.5-5.1 5.1-2.5.6z" strokeWidth="1.5" />
        </svg>
      );
    case "validation":
      return (
        <svg {...shared}>
          <path d="M12 2.5 20 6v5.5c0 5-3.3 8.2-8 10-4.7-1.8-8-5-8-10V6z" />
          <path d="m8 12 2.6 2.6L16.5 9" strokeWidth="2.2" />
        </svg>
      );
  }
}
