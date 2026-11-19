import { Fragment, useId, useMemo, useState } from "react";
import type { Task } from "./types";
import "./TaskLinkPicker.css";

type Props = {
  label: string;
  values: string[];
  onChange: (values: string[]) => void;
  tasks: Task[];
  excludeId?: string;
  allowEpics?: boolean;
  onSearchStart?: () => void;
};

export function TaskLinkPicker({ label, values, onChange, tasks, excludeId, allowEpics = true, onSearchStart }: Props) {
  const inputId = useId();
  const resultsId = useId();
  const [search, setSearch] = useState("");
  const [focused, setFocused] = useState(false);
  const [highlighted, setHighlighted] = useState(0);

  const taskById = useMemo(() => new Map(tasks.map((task) => [task.id, task])), [tasks]);
  const prettyId = (id: string) => {
    const hash = id.lastIndexOf("#");
    if (hash < 0) return id;
    const board = id.slice(0, hash).split(/[\\/]/).filter(Boolean);
    // The file is at <project>/Tasks; show the project, not a drive path.
    return `${board[board.length - 2] || "Project"} / ${id.slice(hash + 1)}`;
  };
  const needle = search.trim().toLowerCase();
  const matches = useMemo(() => {
    if (!needle) return [];
    const ranked = tasks
      .filter((task) => task.id !== excludeId && !values.includes(task.id) &&
        (allowEpics || task.kind !== "epic"))
      .map((task) => {
        const id = task.id.toLowerCase();
        const title = task.title.toLowerCase();
        const idWithoutPrefix = id.split("#").pop()?.replace(/^[a-z][a-z0-9]*-/, "") || id;
        const rank = id === needle || idWithoutPrefix === needle ? 0
          : id.startsWith(needle) || idWithoutPrefix.startsWith(needle) ? 1
          : id.includes(needle) ? 2
          : title.startsWith(needle) ? 3
          : title.includes(needle) ? 4 : -1;
        return { task, rank };
      })
      .filter((result) => result.rank >= 0)
      .sort((a, b) => a.rank - b.rank ||
        Number(a.task.archived) - Number(b.task.archived) ||
        a.task.id.localeCompare(b.task.id));
    // Local IDs are plain task IDs; cross-board links use <board path>#<task ID>.
    // Reserve a few result slots for other boards without letting them outrank locals.
    const local = ranked.filter(({ task }) => !task.id.includes("#"));
    const remote = ranked.filter(({ task }) => task.id.includes("#"));
    const localLimit = remote.length ? 5 : 8;
    const selectedLocal = local.slice(0, localLimit);
    return [...selectedLocal, ...remote.slice(0, 8 - selectedLocal.length)]
      .map(({ task }) => task);
  }, [tasks, values, excludeId, allowEpics, needle]);
  const localMatchCount = matches.filter((task) => !task.id.includes("#")).length;

  const select = (id: string) => {
    if (!values.includes(id)) onChange([...values, id]);
    setSearch("");
    setHighlighted(0);
    setFocused(true);
    document.getElementById(inputId)?.focus();
  };

  return (
    <div className="task-link-field">
      <label htmlFor={inputId} className="task-link-label">{label}</label>
      <div className="task-link-picker">
        <div className="task-link-control" onClick={() => document.getElementById(inputId)?.focus()}>
          {values.map((id) => (
            <span className="task-link-chip" key={id} title={taskById.get(id)?.title || id}>
              <span>{prettyId(id)}</span>
              <button type="button" title={`Remove ${id}`} aria-label={`Remove ${id}`}
                onClick={(event) => {
                  event.stopPropagation();
                  onChange(values.filter((value) => value !== id));
                }}>×</button>
            </span>
          ))}
          <input
            id={inputId}
            autoComplete="off"
            spellCheck={false}
            placeholder={values.length ? "Add another task…" : "Search task ID or title…"}
            value={search}
            role="combobox"
            aria-autocomplete="list"
            aria-expanded={focused && needle.length > 0}
            aria-controls={resultsId}
            aria-activedescendant={focused && matches[highlighted]
              ? `${resultsId}-${matches[highlighted].id}` : undefined}
            onFocus={() => { setFocused(true); onSearchStart?.(); }}
            onBlur={(event) => {
              if (!event.currentTarget.closest(".task-link-picker")?.contains(event.relatedTarget as Node | null)) {
                setFocused(false);
              }
            }}
            onChange={(event) => { setSearch(event.currentTarget.value); setHighlighted(0); }}
            onKeyDown={(event) => {
              if (event.key === "Escape" && focused && needle) {
                event.preventDefault();
                event.stopPropagation();
                setSearch("");
                return;
              }
              if (event.key === "Backspace" && !search && values.length) {
                onChange(values.slice(0, -1));
                return;
              }
              if (!matches.length || !focused) return;
              if (event.key === "ArrowDown") {
                event.preventDefault();
                setHighlighted((index) => (index + 1) % matches.length);
              } else if (event.key === "ArrowUp") {
                event.preventDefault();
                setHighlighted((index) => (index + matches.length - 1) % matches.length);
              } else if (event.key === "Enter") {
                event.preventDefault();
                select(matches[Math.min(highlighted, matches.length - 1)].id);
              }
            }}
          />
        </div>
        {focused && needle && (
          <div className="task-link-results" id={resultsId} role="listbox" aria-label={label}>
            {matches.length ? <>
              {localMatchCount > 0 && <div className="task-link-group-heading">This board</div>}
              {matches.map((task, index) => (
                <Fragment key={task.id}>
                  {index === localMatchCount && <div className={`task-link-group-heading${localMatchCount ? " cross-board" : ""}`}>
                    Other project boards
                  </div>}
                  <button type="button" role="option"
                    id={`${resultsId}-${task.id}`}
                    aria-selected={index === highlighted}
                    className={`task-link-result ${index === highlighted ? "highlighted" : ""}`}
                    onMouseEnter={() => setHighlighted(index)}
                    onMouseDown={(event) => event.preventDefault()}
                    onClick={() => select(task.id)}>
                    <span className="task-link-result-id">{prettyId(task.id)}</span>
                    <span className="task-link-result-title">{task.title}</span>
                    {task.archived && <span className="task-link-result-archived">Archived</span>}
                  </button>
                </Fragment>
              ))}
            </> : <div className="task-link-empty">No matching tasks</div>}
          </div>
        )}
      </div>
    </div>
  );
}
