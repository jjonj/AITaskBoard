import type { TaskPrepfileSettings } from "./types";

/** Prepfile intent is distinct from the older immediate "Prepare" session flow. */
const PREFILE_MARKER = /<!-- taskboard:prepfile-(?:request|target|generator|depth):[^>]*-->/g;
const UUID = /^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i;
const marker = (name: string, notes: string) =>
  notes.match(new RegExp("<!-- taskboard:prepfile-" + name + ":([^>\\s]+) -->"))?.[1] ?? null;

export function prepfileRequestId(notes: string): string | null {
  const id = marker("request", notes);
  return id && UUID.test(id) ? id : null;
}

export function readTaskPrepfile(notes: string): TaskPrepfileSettings | null {
  if (!prepfileRequestId(notes)) return null;
  const generator = marker("generator", notes);
  const depth = marker("depth", notes);
  if (generator !== "chatgpt" && generator !== "luna" && generator !== "haiku") return null;
  if (depth !== "light" && depth !== "medium" && depth !== "heavy") return null;
  return { generator, depth };
}

export function prepfileTargetColumn(notes: string) {
  return marker("target", notes);
}

export function stripTaskPrepfile(notes: string) {
  return notes.replace(PREFILE_MARKER, "").trim();
}

export function writeTaskPrepfile(notes: string, requestId: string, target: string, settings: TaskPrepfileSettings) {
  if (!UUID.test(requestId) || !/^[a-zA-Z][a-zA-Z0-9_-]*$/.test(target))
    throw new Error("Invalid prepfile task identity or destination column");
  return [
    stripTaskPrepfile(notes),
    `<!-- taskboard:prepfile-request:${requestId} -->`,
    `<!-- taskboard:prepfile-target:${target} -->`,
    `<!-- taskboard:prepfile-generator:${settings.generator} -->`,
    `<!-- taskboard:prepfile-depth:${settings.depth} -->`,
  ].filter(Boolean).join("\n\n");
}
