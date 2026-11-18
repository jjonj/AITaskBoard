import { useState } from "react";
import "./InboxCheckboxTable.css";

export function parseCheckboxTable(prompt: string) {
  const rows = prompt.split(/\r?\n/).filter(line => line.includes("|") && /\[\s*\]/.test(line)).map(line => {
    const cells = line.trim().replace(/^\|/, "").replace(/\|$/, "").split("|").map(s => s.trim());
    return { question: cells[0], choices: cells.slice(1).filter(s => /\[\s*\]/.test(s)).map(s => s.replace(/\[\s*\]/g, "").trim()), custom: cells.some(s => /\[custom answer\]/i.test(s)) };
  }).filter(row => row.question && (row.choices.length || row.custom));
  return rows.length ? rows : null;
}
export function InboxCheckboxTable({ prompt, disabled, onAnswer }: {prompt: string; disabled: boolean; onAnswer: (answer: string) => void}) {
  const rows = parseCheckboxTable(prompt) ?? [];
  const [answers, setAnswers] = useState<Record<number, string[]>>({});
  const [custom, setCustom] = useState<Record<number, string>>({});
  const change = (next: Record<number, string[]>, extra: Record<number, string>) => {
    setAnswers(next); setCustom(extra);
    onAnswer(rows.map((row, i) => {
      const values = [...(next[i] ?? []), ...(extra[i]?.trim() ? [extra[i].trim()] : [])];
      return values.length ? row.question + ": " + values.join("; ") : "";
    }).filter(Boolean).join("\n"));
  };
  return <div className="inbox-checkbox-table"><div className="inbox-checkbox-table-scroll"><table>
    <thead><tr><th>Question</th><th>Options (select multiple)</th><th>Custom answer</th></tr></thead>
    <tbody>{rows.map((row, i) => <tr key={i}><th scope="row">{row.question}</th>
      <td><div className="inbox-checkbox-choices">{row.choices.map(choice =>
        <label key={choice}><input type="checkbox" disabled={disabled} checked={(answers[i] ?? []).includes(choice)}
          onChange={e => change({...answers, [i]: e.target.checked ? [...(answers[i] ?? []), choice] : (answers[i] ?? []).filter(v => v !== choice)}, custom)} />{choice}</label>)}</div></td>
      <td>{row.custom && <input aria-label={row.question + " custom answer"} placeholder="Other…" disabled={disabled}
        value={custom[i] ?? ""} onChange={e => change(answers, {...custom, [i]: e.target.value})} />}</td>
    </tr>)}</tbody></table></div><small>Submit all selected answers together.</small></div>;
}
