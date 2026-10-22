export type GeometryRect = { x: number; y: number; width: number; height: number };
export type ColumnRectangle = { id: string; rect: GeometryRect };

/** Rectangles that should cover an edge between two task columns.
 * Source/destination columns are never occluders. Returned bounds include
 * three pixels of safety margin for the SVG stroke glow.
 */
export function interveningColumns(
  source: ColumnRectangle,
  destination: ColumnRectangle,
  columns: readonly ColumnRectangle[],
): GeometryRect[] {
  const midX = (c: ColumnRectangle) => c.rect.x + c.rect.width / 2;
  const minX = Math.min(midX(source), midX(destination));
  const maxX = Math.max(midX(source), midX(destination));
  return columns.filter((c) => c.id !== source.id && c.id !== destination.id)
    .filter((c) => midX(c) > minX && midX(c) < maxX)
    .map((c) => ({
      x: c.rect.x - 3, y: c.rect.y - 3,
      width: c.rect.width + 6, height: c.rect.height + 6,
    }));
}

/** Geometry only: never adds padding to the DOM or alters card widths. */
export function crossColumnCurve(sx: number, sy: number, tx: number, ty: number, direction: 1 | -1): string {
  const tangent = Math.min(170, Math.abs(tx - sx) * 0.38);
  const f = (x: number) => x.toFixed(1);
  return `M ${f(sx)} ${f(sy)} C ${f(sx + direction * tangent)} ${f(sy)}, ${f(tx - direction * tangent)} ${f(ty)}, ${f(tx)} ${f(ty)}`;
}
