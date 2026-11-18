import type { ChangeEvent } from "react";
import "./InboxCheckboxTable.css";

export type CheckboxTableRow = {
  question: string;
  choices: string[];
  custom: boolean;
  /** Extra instructions between question rows. */
  leading?: string;
};
export type CheckboxTable = {
  rows: CheckboxTableRow[];
  before: string;
  after: string;
};
export type CheckboxAnswers = Record<number, { choices: string[]; custom: string }>;

const checkbox = /\[\s*\]/;
const customMarker = /\[\s*(?:custom(?: answer)?|other(?: answer)?|write[- ]?in)\s*\]/i;

function parseRow(line: string): CheckboxTableRow | null {
  if (!line.includes("|")) return null;
  const cells = line.trim().replace(/^\|/, "").replace(/\|$/, "").split(/(?<!\\)\|/)
    .map(cell => cell.replace(/\\\|/g, "|").trim());
  if (cells.length < 2 || !cells[0]) return null;
  const choices = cells.slice(1).filter(cell => checkbox.test(cell) && !customMarker.test(cell))
    .map(cell => cell.replace(/\[\s*\]/g, "").trim()).filter(Boolean);
  const custom = cells.slice(1).some(cell => customMarker.test(cell));
  if (!choices.length && !custom) return null;
  return { question: cells[0], choices, custom };
}

function isMarkdownSeparator(line: string): boolean {
  return /^\s*\|?\s*:?-{3,}:?\s*(?:\|\s*:?-{3,}:?\s*)+\|?\s*$/.test(line);
}

/**
 * The AI can send a multi-question pipe table directly in the ask-user prompt:
 * Q1 | Choice A [ ] | Choice B [] | [custom answer]
 * Regular Markdown tables (header and separator) are also accepted.
 * Non-table instructions before/after are retained for the Inbox.
 */
export function parseCheckboxTable(prompt: string): CheckboxTable | null {
  const lines = prompt.split(/\r?\n/);
  const indexed = lines.flatMap((line, index) => {
    const row = parseRow(line);
    return row ? [{ row, index }] : [];
  });
  if (!indexed.length) return null;
  let start = indexed[0].index;
  const end = indexed[indexed.length - 1].index;
  if (start > 0 && isMarkdownSeparator(lines[start - 1])) {
    start--;
    if (start > 0 && lines[start - 1].includes("|")) start--;
  }
  return {
    rows: indexed.map((entry, i) => {
      if (!i) return entry.row;
      const between = lines.slice(indexed[i - 1].index + 1, entry.index);
      const leading = between.filter((line, position) =>
        !isMarkdownSeparator(line) &&
        !(line.includes("|") && isMarkdownSeparator(between[position + 1] ?? "")))
        .join("\n").trim();
      return leading ? { ...entry.row, leading } : entry.row;
    }),
    before: lines.slice(0, start).join("\n").trim(),
    after: lines.slice(end + 1).join("\n").trim(),
  };
}

export function checkboxRowCount(rows: CheckboxTableRow[], answers: CheckboxAnswers): number {
  return rows.filter((_, index) => {
    const value = answers[index];
    return !!value && (value.choices.length > 0 || !!value.custom.trim());
  }).length;
}

/** Include unanswered rows explicitly; no AI question is silently discarded. */
export function formatCheckboxAnswers(rows: CheckboxTableRow[], answers: CheckboxAnswers): string {
  return rows.map((row, index) => {
    const value = answers[index];
    const selected = [...(value?.choices ?? []), ...(value?.custom.trim() ? [value.custom.trim()] : [])];
    return row.question + ": " + (selected.length ? selected.join("; ") : "(not answered)");
  }).join("\n");
}

export function InboxCheckboxTable({
  table, answers, disabled, onChange,
}: {
  table: CheckboxTable;
  answers: CheckboxAnswers;
  disabled: boolean;
  onChange: (next: CheckboxAnswers) => void;
}) {
  const changeChoice = (index: number, choice: string, event: ChangeEvent<HTMLInputElement>) => {
    const row = answers[index] ?? { choices: [], custom: "" };
    onChange({
      ...answers,
      [index]: {
        ...row,
        choices: event.target.checked
          ? [...row.choices, choice]
          : row.choices.filter(value => value !== choice),
      },
    });
  };
  const changeCustom = (index: number, text: string) => {
    const row = answers[index] ?? { choices: [], custom: "" };
    onChange({ ...answers, [index]: { ...row, custom: text } });
  };
  const answered = checkboxRowCount(table.rows, answers);
  const hasCustom = table.rows.some(row => row.custom);

  return <section className="inbox-checkbox-table" aria-label="Multiple-choice questionnaire">
    <div className="inbox-checkbox-table-top">
      <strong>Questions</strong>
      <span aria-live="polite">{answered} of {table.rows.length} answered</span>
    </div>
    <div className="inbox-checkbox-table-scroll">
      <table>
        <thead><tr><th scope="col">Question</th><th scope="col">Select any that apply</th>
          {hasCustom && <th scope="col">Custom answer</th>}</tr></thead>
        <tbody>{table.rows.map((row, index) =>
          <tr key={index}>
            <th scope="row">{row.leading && <small className="inbox-checkbox-row-note">{row.leading}</small>}{row.question}</th>
            <td>
              <div className="inbox-checkbox-choices">{row.choices.map((choice, optionIndex) =>
                <label key={optionIndex}>
                  <input type="checkbox" disabled={disabled}
                    checked={(answers[index]?.choices ?? []).includes(choice)}
                    onChange={event => changeChoice(index, choice, event)} />
                  <span>{choice}</span>
                </label>)}</div>
            </td>
            {hasCustom && <td>
              {row.custom && <input type="text" aria-label={row.question + ": custom answer"}
                placeholder="Type your answer…" disabled={disabled}
                value={answers[index]?.custom ?? ""}
                onChange={event => changeCustom(index, event.target.value)} />}
            </td>}
          </tr>)}
        </tbody>
      </table>
    </div>
    <div className="inbox-checkbox-table-bottom">
      {answered < table.rows.length
        ? "Unanswered rows will be reported as “(not answered)”."
        : "All questions answered."}
    </div>
  </section>;
}

