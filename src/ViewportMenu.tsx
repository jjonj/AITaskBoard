import { useLayoutEffect, useRef, type CSSProperties, type HTMLAttributes } from "react";

const VIEWPORT_GAP = 8;

type ViewportMenuProps = HTMLAttributes<HTMLDivElement> & {
  x: number;
  y: number;
  style?: CSSProperties;
};

// Position against the menu's *rendered* size, not an estimated row count.
// CSS caps the height and provides scrolling for menus taller than the window.
export function ViewportMenu({ x, y, style, children, ...props }: ViewportMenuProps) {
  const ref = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const menu = ref.current;
    if (!menu) return;

    const place = () => {
      const rect = menu.getBoundingClientRect();
      menu.style.left = `${Math.max(VIEWPORT_GAP, Math.min(x, window.innerWidth - rect.width - VIEWPORT_GAP))}px`;
      menu.style.top = `${Math.max(VIEWPORT_GAP, Math.min(y, window.innerHeight - rect.height - VIEWPORT_GAP))}px`;
    };

    place(); // Before paint, including when reopening on a different task.
    const observer = new ResizeObserver(place);
    observer.observe(menu);
    window.addEventListener("resize", place);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", place);
    };
  }, [x, y]);

  return <div {...props} ref={ref} style={{ ...style, left: x, top: y }}>{children}</div>;
}
