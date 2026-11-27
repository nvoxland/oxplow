import { createProject, openProject } from "../api.js";
import { pickFolder } from "../tauri-bridge/nativeDialog.js";
import { recordOpError } from "../opErrors.js";

/// Prompt for a folder and open it as an **existing** project.
/// `newWindow=false` replaces the current window (this process exits
/// once the new one spawns); `true` opens an additional independent
/// window. A folder that isn't a project yet is refused by
/// `open_project` — File ▸ New Project… is the door for that.
export async function pickAndOpenProject(newWindow: boolean) {
  const selected = await pickFolder(newWindow ? "Open Project in New Window" : "Open Project");
  if (selected === null) return;
  try {
    await openProject(selected, newWindow);
  } catch (e) {
    recordOpError({
      label: "Open project",
      message: e instanceof Error ? e.message : String(e),
    });
  }
}

/// Prompt for a folder and create a new project in it. Always opens in
/// a new window, so creating never closes the window the command ran
/// from; a folder that already is a project is refused by
/// `create_project`.
export async function pickAndCreateProject() {
  const selected = await pickFolder("New Project");
  if (selected === null) return;
  try {
    await createProject(selected);
  } catch (e) {
    recordOpError({
      label: "New project",
      message: e instanceof Error ? e.message : String(e),
    });
  }
}
