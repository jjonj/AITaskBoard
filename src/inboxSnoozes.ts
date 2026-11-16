/** Browser-local inbox snoozes are keyed by board path and task ID. */
export const readInboxSnoozes = (): Record<string, number> => {
  try {
    const value: unknown = JSON.parse(window.localStorage.getItem("taskboard.inboxSnoozes") || "{}");
    if (!value || typeof value !== "object" || Array.isArray(value)) return {};
    return value as Record<string, number>;
  } catch { return {}; }
};

export const isInboxItemSnoozed = (
  snoozes: Record<string, number>, path: string, taskId: string, now = Date.now(),
): boolean => Number(snoozes[`${path}|${taskId}`]) > now;
