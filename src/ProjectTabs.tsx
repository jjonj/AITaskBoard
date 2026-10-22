import { useEffect, useState } from "react";
import { activateProject, addProject, listProjects, removeProject, showMainWindow } from "./api";
import type { ProjectCatalog, ProjectTab } from "./api";

type Props = {
  Board: (props: { project: ProjectTab; onInteractionChange: (locked: boolean) => void }) => React.ReactNode;
};

export function ProjectTabs({ Board }: Props) {
  const [catalog, setCatalog] = useState<ProjectCatalog | null>(null);
  const [locked, setLocked] = useState(false);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState("");
  const [newProject, setNewProject] = useState(false);
  const [name, setName] = useState("");
  const [path, setPath] = useState("");
  const [createBoard, setCreateBoard] = useState(false);

  useEffect(() => {
    let mounted = true;
    void listProjects().then((value) => {
      if (mounted) setCatalog(value);
    }).catch((err: unknown) => {
      if (mounted) setError(String(err));
      // The native window starts hidden; even a corrupted project catalog
      // must display an actionable error instead of an invisible window.
      void showMainWindow().catch(() => undefined);
    });
    return () => { mounted = false; };
  }, []);

  const switchTo = async (id: string) => {
    if (working || locked || newProject || !catalog || catalog.activeId === id) return;
    setWorking(true);
    setError("");
    try {
      // Persist selection BEFORE unmounting the previous workspace. If
      // persistence fails, keep displaying the current board unchanged.
      const updated = await activateProject(id);
      setLocked(false);
      setCatalog(updated);
    } catch (err) {
      setError(String(err));
    } finally {
      setWorking(false);
    }
  };

  const add = async () => {
    if (working || locked || !name.trim() || !path.trim()) return;
    setWorking(true);
    setError("");
    try {
      const updated = await addProject(name, path, createBoard);
      setCatalog(updated);
      setLocked(false);
      setNewProject(false);
      setName("");
      setPath("");
      setCreateBoard(false);
    } catch (err) {
      setError(String(err));
    } finally {
      setWorking(false);
    }
  };

  const remove = async (project: ProjectTab) => {
    if (working || locked || !catalog || catalog.projects.length < 2) return;
    if (!window.confirm(`Remove "${project.name}" from TaskBoard tabs?\n\nThe board folder and all Markdown tasks will be left untouched.`)) return;
    setWorking(true);
    setError("");
    try {
      const updated = await removeProject(project.id);
      setLocked(false);
      setCatalog(updated);
    } catch (err) {
      setError(String(err));
    } finally {
      setWorking(false);
    }
  };

  const active = catalog?.projects.find((project) => project.id === catalog.activeId);
  return (
    <div className="project-shell">
      <div className="project-tabs" role="tablist" aria-label="Project task boards">
        {catalog?.projects.map((project) => (
          <div className={`project-tab-wrapper ${project.id === catalog.activeId ? "selected" : ""}`}
            key={project.id}>
            <button
              type="button"
              role="tab"
              aria-selected={project.id === catalog.activeId}
              aria-controls="project-board"
              className="project-tab"
              title={project.path}
              disabled={working || locked || newProject}
              onClick={() => void switchTo(project.id)}
            >{project.name}</button>
            {catalog.projects.length > 1 && (
              <button type="button" className="project-tab-close"
                aria-label={`Remove ${project.name} tab`}
                title="Remove tab only — never delete project files"
                disabled={working || locked || newProject}
                onClick={() => void remove(project)}>×</button>
            )}
          </div>
        ))}
        <button type="button" className="project-tab-add"
          title="Open another project's task board"
          disabled={working || locked || newProject || !catalog}
          onClick={() => { setNewProject(true); setError(""); }}>
          + Project
        </button>
        {locked && <span className="project-switch-note" title="Finish or cancel the current task operation before switching projects">
          Close the editor to change project
        </span>}
      </div>
      {error && <div className="project-catalog-error" role="alert">{error}</div>}
      {active && <div className="project-workspace" id="project-board" role="tabpanel">
        <Board key={active.id} project={active} onInteractionChange={setLocked} />
      </div>}
      {!active && <div className="project-loading">Loading TaskBoard project tabs…</div>}
      {newProject && (
        <div className="modal-backdrop project-add-backdrop"
          onMouseDown={() => { if (!working) setNewProject(false); }}>
          <section className="project-add-dialog" role="dialog" aria-modal="true" aria-labelledby="project-add-heading"
            onMouseDown={(event) => event.stopPropagation()}>
            <h2 id="project-add-heading">Add project</h2>
            <p>Open a separate Markdown task board in its own project tab. Existing tasks are not moved or copied.</p>
            <label>Project tab name
              <input autoFocus maxLength={80} value={name} placeholder="e.g. Altraea"
                onChange={(event) => setName(event.currentTarget.value)} />
            </label>
            <label>Board folder (absolute path)
              <input value={path} placeholder="D:\\Projects\\MyProject\\Tasks"
                onChange={(event) => setPath(event.currentTarget.value)} />
            </label>
            <label className="project-create-toggle">
              <input type="checkbox" checked={createBoard}
                onChange={(event) => setCreateBoard(event.currentTarget.checked)} />
              Create a new TaskBoard board in this existing folder if none exists
            </label>
            <p className="project-add-hint">
              For an existing board, select the folder containing <code>_board.md</code>.
              The folder must already exist. New boards create <code>_board.md</code>,
              <code> Active/</code> and <code> Archive/</code> inside it.
            </p>
            <div className="project-dialog-actions">
              <button type="button" className="button secondary" disabled={working}
                onClick={() => setNewProject(false)}>Cancel</button>
              <button type="button" className="button primary"
                disabled={working || !name.trim() || !path.trim()} onClick={() => void add()}>
                {working ? "Adding…" : "Add project"}
              </button>
            </div>
          </section>
        </div>
      )}
    </div>
  );
}
