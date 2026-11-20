import type { Column, Task } from "./types";

type Mode = { mode: string };
export type DependencyBoardState = {
  columns: Column[];
  executionModes: Record<string, Mode> | null;
};
type Input = {
  tasks: Task[];
  columns: Column[];
  executionModes: Record<string, Mode>;
  pendingTaskQueue: Record<string, boolean>;
  pendingColumnQueue: Record<string, boolean>;
  remoteTasks: Map<string, Task>;
  remoteBoards: Map<string, DependencyBoardState>;
  claimStates: Record<string, { agentState: string }>;
};
export type DependencyWaitStatus = { stalled: boolean; reasons: string[] };

// A WAIT count describes unfinished prerequisites; this diagnoses whether
// those prerequisites (and the dependent task) can advance without intervention.
export function createDependencyWaitInspector(input: Input) {
  const local = new Map(input.tasks.map(task => [task.id, task]));
  const localTerminal = new Set(input.columns.filter(column => column.terminal).map(column => column.id));
  const reasons = (task: Task): DependencyWaitStatus => {
    const problems = new Set<string>();
    const visited = new Set<string>();
    const walk = (id: string, path: Set<string>) => {
      if (path.has(id)) {
        problems.add(`${id}: circular dependency (manual repair required)`);
        return;
      }
      if (visited.has(id)) return;
      visited.add(id);
      const separator = id.lastIndexOf("#");
      const remotePath = separator >= 0 ? id.slice(0, separator) : null;
      const shortId = remotePath ? id.slice(separator + 1) : id;
      const item = remotePath ? input.remoteTasks.get(id) : local.get(id);
      if (!item) {
        problems.add(`${id}: prerequisite missing or its board unavailable`);
        return;
      }
      const board = remotePath ? input.remoteBoards.get(remotePath) : null;
      const columns = remotePath ? board?.columns : input.columns;
      const column = columns?.find(candidate => candidate.id === item.column);
      if (item.archived || item.completed || (remotePath ? !!column?.terminal : localTerminal.has(item.column))) return;
      const label = remotePath ? id : item.id;
      const pendingQuestion = item.attention.includes("question") && !!item.question && !item.question.answer;
      if (item.column === "draft") problems.add(`${label}: in User Drafts — move it to an executable column`);
      else if (item.kind === "for_user" || item.attention.includes("needs-user")) problems.add(`${label}: requires your action`);
      else if (pendingQuestion) problems.add(`${label}: waiting for your answer`);
      else if (item.column === "blocked" || item.attention.includes("blocked"))
        problems.add(`${label}: technically blocked${item.blocked_reason ? ` — ${item.blocked_reason.trim()}` : ""}`);
      else if (item.review_status === "open" || item.review_status === "awaiting_revision")
        problems.add(`${label}: waiting for a review or revision`);
      else if (item.attention.includes("prepare"))
        problems.add(`${label}: preparation requested before implementation`);
      else if (item.claimed_by) {
        const state = remotePath ? undefined : input.claimStates[item.id]?.agentState;
        if (state && /errored|failed|dead|unavailable|stopped/i.test(state))
          problems.add(`${label}: claimed agent is ${state}`);
      } else if (!column || !column.ai_ready || column.advisory) {
        problems.add(`${label}: in ${column?.name ?? item.column}, which is not AI-ready`);
      } else {
        const mode = remotePath ? board?.executionModes?.[shortId]?.mode
          : input.executionModes[shortId]?.mode;
        const pending = remotePath ? undefined
          : input.pendingTaskQueue[shortId] ?? input.pendingColumnQueue[item.column];
        const queued = pending ?? (mode ? mode === "queue" : !!column.auto_queue);
        if (!queued) problems.add(`${label}: not queued (will not start automatically)`);
      }
      const next = new Set(path);
      next.add(id);
      for (const prerequisite of item.depends_on ?? []) {
        const target = separator >= 0 && !prerequisite.includes("#")
          ? `${remotePath}#${prerequisite}` : prerequisite;
        walk(target, next);
      }
    };
    walk(task.id, new Set());
    return { stalled: problems.size > 0, reasons: [...problems] };
  };
  return reasons;
}
