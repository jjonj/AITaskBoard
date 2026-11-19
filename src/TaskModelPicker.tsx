import { useEffect, useMemo, useState } from "react";
import { taskModelFavorites } from "./api";
import { taskModelKey } from "./taskModels";
import type { TaskModelSelection } from "./types";
import "./taskModels.css";

type Favorite = { provider: string; model: string };

export function TaskModelPicker({
  value, onChange, caption = "Task model",
}: {
  value: TaskModelSelection | null;
  onChange: (next: TaskModelSelection | null) => void;
  caption?: string;
}) {
  const [favorites, setFavorites] = useState<Favorite[]>([]);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let mounted = true;
    const read = () => void taskModelFavorites().then(rows => {
      if (mounted) { setFavorites(rows); setError(null); }
    }).catch(err => {
      if (mounted) setError(String(err));
    });
    read();
    const timer = window.setInterval(read, 4000);
    return () => { mounted = false; window.clearInterval(timer); };
  }, []);

  const options = useMemo(() => {
    const rows = new Map(favorites.map(row => [taskModelKey(row.provider, row.model), row]));
    if (value) rows.set(taskModelKey(value.provider, value.model), {
      provider: value.provider, model: value.model,
    });
    return [...rows.entries()];
  }, [favorites, value]);

  return <div className="task-model-picker">
    <label>
      <span>{caption}</span>
      <select aria-label={caption}
        value={value ? taskModelKey(value.provider, value.model) : "chatgpt"}
        onChange={event => {
          if (event.currentTarget.value === "chatgpt") return onChange(null);
          const row = options.find(([key]) => key === event.currentTarget.value)?.[1];
          if (row) onChange({ ...row, effort: "medium" });
        }}>
        <option value="chatgpt">ChatGPT (browser session)</option>
        {options.map(([key, row]) =>
          <option key={key} value={key}>
            {row.model} ({row.provider}){favorites.some(f => taskModelKey(f.provider, f.model) === key) ? "" : " - no longer favorited"}
          </option>)}
      </select>
    </label>
    {value && <label>
      <span>Effort</span>
      <select aria-label="Task model effort" value={value.effort}
        onChange={event => onChange({ ...value, effort: event.currentTarget.value as TaskModelSelection["effort"] })}>
        <option value="low">Low</option>
        <option value="medium">Medium</option>
        <option value="high">High</option>
        <option value="xhigh">Extra high</option>
      </select>
    </label>}
    {error && <small title={error}>T3 favorites unavailable; saved selections remain usable.</small>}
  </div>;
}
