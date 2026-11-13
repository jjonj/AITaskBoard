import type { TaskDraft } from "./types";

// "blocked" entries are written only by task-core when an agent blocks a task.
export type DescriptionUpdateKind = "update" | "user-note" | "legacy-ai" | "blocked";
export type DescriptionUpdate = {
  kind: DescriptionUpdateKind;
  at: string;
  text: string;
};

export const AUTHORIZE_UNBLOCK_TEXT = "Agents have full authority to do what it takes to unblock this task.";

const START = /^<!-- taskboard:update:(update|user-note|legacy-ai|blocked)\|([^\n]+) -->$/;
const END = "<!-- taskboard:end-update -->";

const quoteLines = (text: string) =>
  text.replace(/\r\n/g, "\n").trim().split("\n").map((line) => line ? `> ${line}` : ">").join("\n");
const unquoteLines = (lines: string[]) =>
  lines.map((line) => line === ">" ? "" : line.slice(2)).join("\n");

/**
 * Description remains a single ordinary Markdown section. Update blocks use
 * HTML-comment boundaries and quoted lines so ## headings in user messages
 * cannot escape the Description section of the Rust Markdown parser.
 *
 * Malformed/hand-edited histories are never discarded: display their entire
 * source as the editable original description until repaired manually.
 */
export function parseDescriptionTimeline(description: string): {
  initial: string;
  updates: DescriptionUpdate[];
} {
  // Keep whitespace while the controlled textarea is being edited. Trimming
  // here swallowed every trailing space before the user could type the next word.
  const normalized = description.replace(/\r\n/g, "\n");
  const lines = normalized.split("\n");
  const first = lines.findIndex((line) => START.test(line));
  if (first < 0) return { initial: normalized, updates: [] };
  // The formatter adds exactly two newlines before the first update block.
  // Remove that separator only, keeping every user-typed space and line break.
  const prefix = lines.slice(0, first).join("\n");
  const initial = prefix.endsWith("\n") ? prefix.slice(0, -1) : prefix;
  const updates: DescriptionUpdate[] = [];
  let i = first;
  while (i < lines.length) {
    while (i < lines.length && lines[i].trim() === "") i++;
    if (i >= lines.length) break;
    const start = lines[i].match(START);
    if (!start || !Number.isFinite(Date.parse(start[2]))) {
      return { initial: normalized, updates: [] };
    }
    i++;
    const body: string[] = [];
    while (i < lines.length && lines[i] !== END) {
      if (!(lines[i] === ">" || lines[i].startsWith("> "))) {
        return { initial: normalized, updates: [] };
      }
      body.push(lines[i]);
      i++;
    }
    if (i === lines.length) return { initial: normalized, updates: [] };
    i++;
    updates.push({
      kind: start[1] as DescriptionUpdateKind,
      at: start[2],
      text: unquoteLines(body),
    });
  }
  return { initial, updates };
}

export function formatDescriptionTimeline(initial: string, updates: DescriptionUpdate[]): string {
  const blocks = updates.map(({ kind, at, text }) =>
    `<!-- taskboard:update:${kind}|${at} -->\n${quoteLines(text)}\n${END}`);
  return initial ? [initial, ...blocks].join("\n\n") : blocks.join("\n\n");
}

export function updateInitialDescription(description: string, newInitial: string): string {
  return formatDescriptionTimeline(newInitial, parseDescriptionTimeline(description).updates);
}

export function appendDescriptionUpdates(
  description: string,
  additions: Array<{ kind: DescriptionUpdateKind; text: string }>,
  at = new Date().toISOString(),
): string {
  const parsed = parseDescriptionTimeline(description);
  const updates = [...parsed.updates, ...additions
    .filter((entry) => entry.text.trim())
    .map((entry) => ({ ...entry, text: entry.text.trim(), at }))];
  return formatDescriptionTimeline(parsed.initial, updates);
}

// Locked descriptions cannot gain image links in their original brief.
export function attachImagesToDescription(description: string, links: string[], locked: boolean): string {
  if (!links.length) return description;
  const images = links.join("\n\n");
  if (locked) return appendDescriptionUpdates(description, [{ kind: "user-note", text: images }]);
  const initial = parseDescriptionTimeline(description).initial;
  return updateInitialDescription(description, [initial, images].filter(Boolean).join("\n\n"));
}

// Legacy NeedsUser remains readable but For user is the sole new human-only
// status. Existing notes are safely moved into the task's Description history.
export function prepareTaskDraftForSave(draft: TaskDraft, pendingUpdate: string, claimedBy?: string | null): TaskDraft {
  const isEpic = draft.kind === "epic";
  const additions: Array<{ kind: DescriptionUpdateKind; text: string }> = [];
  if (draft.ai_notes.trim()) additions.push({ kind: "legacy-ai", text: draft.ai_notes });
  if (pendingUpdate.trim()) additions.push({ kind: "update", text: pendingUpdate });
  if (!isEpic && draft.user_notes.trim()) additions.push({ kind: "user-note", text: draft.user_notes });
  const hasUserNote = !isEpic && !!draft.user_notes.trim();
  const attention = draft.attention.filter((flag) => flag !== "needs-user" || (!!claimedBy && !hasUserNote));
  if (hasUserNote && !attention.includes("user-note")) attention.push("user-note");
  // Line-list fields keep raw lines while typing; normalize them here.
  const cleanLines = (lines: string[] = []) => lines.map((line) => line.trim()).filter(Boolean);
  return {
    ...draft,
    acceptance_criteria: cleanLines(draft.acceptance_criteria),
    question: draft.question && { ...draft.question, options: cleanLines(draft.question.options) },
    attention,
    kind: !isEpic && !claimedBy && !hasUserNote && draft.kind === "task" && draft.attention.includes("needs-user")
      ? "for_user" : draft.kind,
    description: appendDescriptionUpdates(draft.description, additions),
    ai_notes: "",
    user_notes: isEpic ? draft.user_notes : "",
  };
}
