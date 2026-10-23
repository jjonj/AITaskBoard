import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { activateProject, addProject, listProjects, removeProject, setProjectFolders, showMainWindow } from "./api";
import type { ProjectCatalog, ProjectTab } from "./api";

type Props = {
  Board: (props: { project: ProjectTab; onInteractionChange: (locked: boolean) => void }) => React.ReactNode;
};
type DialogMode = "create" | "folders" | null;
const OBSIDIAN_ROOT = "B:\\GDrive\\SharedWithPhone\\Obsidian\\jjj";

const suggestedName = (path: string) => {
  const parts = path.trim().replace(/[\\/]+$/, "").split(/[\\/]/).filter(Boolean);
  if (parts[parts.length - 1]?.toLowerCase() === "tasks") parts.pop();
  return parts[parts.length - 1] ?? "";
};

export function ProjectTabs({ Board }: Props) {
  const [catalog, setCatalog] = useState<ProjectCatalog | null>(null);
  const [locked, setLocked] = useState(false);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState("");
  const [dialogMode, setDialogMode] = useState<DialogMode>(null);
  const [editingId, setEditingId] = useState("");
  const [name, setName] = useState("");
  const [nameEdited, setNameEdited] = useState(false);
  const [folders, setFolders] = useState<string[]>([""]);

  useEffect(() => {
    let mounted = true;
    void listProjects().then((value) => {
      if (mounted) setCatalog(value);
    }).catch((err: unknown) => {
      if (mounted) setError(String(err));
      // Show an actionable error even when the native catalog cannot load.
      void showMainWindow().catch(() => undefined);
    });
    return () => { mounted = false; };
  }, []);

  const switchTo = async (id: string) => {
    if (working || locked || dialogMode || !catalog || catalog.activeId === id) return;
    setWorking(true);
    setError("");
    try {
      const updated = await activateProject(id);
      setLocked(false);
      setCatalog(updated);
    } catch (err) {
      setError(String(err));
    } finally {
      setWorking(false);
    }
  };

  const startCreate = () => {
    setName("");
    setNameEdited(false);
    setFolders([""]);
    setEditingId("");
    setError("");
    setDialogMode("create");
  };

  const startEditFolders = (project: ProjectTab) => {
    if (locked || working) return;
    setEditingId(project.id);
    setName(project.name);
    setFolders(project.folders?.length ? [...project.folders] : [""]);
    setError("");
    setDialogMode("folders");
  };

  const updateFolder = (index: number, value: string) => {
    setFolders((current) => current.map((old, i) => i === index ? value : old));
    if (dialogMode === "create" && index === 0 && !nameEdited) setName(suggestedName(value));
  };

  const browseFolder = async (index: number) => {
    if (working) return;
    setWorking(true);
    setError("");
    try {
      const path = await open({ directory: true, multiple: false, title: "Select a project folder" });
      if (typeof path === "string") updateFolder(index, path);
    } catch (err) {
      setError(String(err));
    } finally {
      setWorking(false);
    }
  };

  const saveDialog = async () => {
    if (working || locked) return;
    if (dialogMode === "create" && !name.trim()) return;
    setWorking(true);
    setError("");
    try {
      const selectedFolders = folders.map((item) => item.trim()).filter(Boolean);
      const updated = dialogMode === "create"
        ? await addProject(name, selectedFolders)
        : await setProjectFolders(editingId, selectedFolders);
      setCatalog(updated);
      setLocked(false);
      setDialogMode(null);
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
              title={`Tasks: ${project.path}\nProject folders: ${(project.folders || []).join("; ") || "(none)"}`}
              disabled={working || locked || !!dialogMode}
              onClick={() => void switchTo(project.id)}
            >{project.name}</button>
            <button type="button" className="project-tab-settings"
              aria-label={`Edit ${project.name} folders`}
              title="Edit project folders"
              disabled={working || locked || !!dialogMode}
              onClick={() => startEditFolders(project)}>⚙</button>
            {catalog.projects.length > 1 && (
              <button type="button" className="project-tab-close"
                aria-label={`Remove ${project.name} tab`}
                title="Remove tab only — never delete project files"
                disabled={working || locked || !!dialogMode}
                onClick={() => void remove(project)}>×</button>
            )}
          </div>
        ))}
        <button type="button" className="project-tab-add"
          title="Add a project with its own TaskBoard tasks"
          disabled={working || locked || !!dialogMode || !catalog}
          onClick={startCreate}>+ Project</button>
        {locked && <span className="project-switch-note">Close the editor to change project</span>}
      </div>
      {error && !dialogMode && <div className="project-catalog-error" role="alert">{error}</div>}
      {active && <div className="project-workspace" id="project-board" role="tabpanel">
        <Board key={active.id} project={active} onInteractionChange={setLocked} />
      </div>}
      {!active && <div className="project-loading">Loading TaskBoard project tabs…</div>}
      {dialogMode && (
        <div className="modal-backdrop project-add-backdrop"
          onMouseDown={() => { if (!working) setDialogMode(null); }}>
          <section className="project-add-dialog" role="dialog" aria-modal="true" aria-labelledby="project-add-heading"
            onMouseDown={(event) => event.stopPropagation()}>
            <h2 id="project-add-heading">{dialogMode === "create" ? "Add project" : `Project folders · ${name}`}</h2>
            <p>Associate one or more folders with this project. Your files stay where they are.</p>
            <div className="project-folder-entries">
              {folders.map((folder, index) => (
                <label key={index}>Project folder {folders.length > 1 ? index + 1 : ""}
                  <div className="project-folder-row">
                    <input autoFocus={index === 0} value={folder}
                      placeholder="Choose a source / working folder"
                      onChange={(event) => updateFolder(index, event.currentTarget.value)} />
                    <button type="button" className="button secondary"
                      disabled={working} onClick={() => void browseFolder(index)}>Browse…</button>
                    {folders.length > 1 && <button type="button" className="button secondary project-folder-remove"
                      aria-label={`Remove folder ${index + 1}`} disabled={working}
                      onClick={() => setFolders((current) => current.filter((_, i) => i !== index))}>×</button>}
                  </div>
                </label>
              ))}
            </div>
            <button className="project-add-folder" type="button" disabled={working}
              onClick={() => setFolders((current) => [...current, ""])}>+ Add folder</button>
            {dialogMode === "create" ? (
              <>
                <label>Project name
                  <input maxLength={80} value={name} placeholder="Suggested from the selected folder"
                    onChange={(event) => { setNameEdited(true); setName(event.currentTarget.value); }} />
                </label>
                <p className="project-add-hint">
                  Tasks are stored automatically in your Obsidian vault:
                  <code className="project-task-path">{OBSIDIAN_ROOT}\{name.trim() || "[ProjectName]"}\Tasks</code>
                  Any existing TaskBoard there will be reused.
                </p>
              </>
            ) : (
              <p className="project-add-hint">TaskBoard tasks remain at
                <code className="project-task-path">{catalog?.projects.find((item) => item.id === editingId)?.path}</code>
                Removing a folder from this list never deletes it.
              </p>
            )}
            {error && <div className="project-form-error" role="alert">{error}</div>}
            <div className="project-dialog-actions">
              <button type="button" className="button secondary" disabled={working}
                onClick={() => { setDialogMode(null); setError(""); }}>Cancel</button>
              <button type="button" className="button primary"
                disabled={working || (dialogMode === "create" && !name.trim())}
                onClick={() => void saveDialog()}>
                {working ? "Saving…" : dialogMode === "create" ? "Add project" : "Save folders"}
              </button>
            </div>
          </section>
        </div>
      )}
    </div>
  );
}